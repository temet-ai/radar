//! Copilot session events into radar's envelope kinds. Pure: the SDK's event arrives as a
//! [`RawEvent`] (the same shape recorded to the fixture), so the mapping is tested
//! without the CLI.

use radar_sessions::event::{
    Durability, EventKind, PermissionKind, PermissionRequest, PermissionTarget, Question,
    QuestionChoice, QuestionReply, QuestionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A Copilot `SessionEvent`, as recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawEvent {
    pub id: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// `ev.ephemeral.unwrap_or(false)`.
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub event_type: String,
    pub data: Value,
}

impl RawEvent {
    pub fn durability(&self) -> Durability {
        if self.ephemeral {
            Durability::Ephemeral
        } else {
            Durability::Persisted
        }
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn cut(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let mut t: String = line.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Copilot's own words for what a turn is, against radar's: radar's turn is the operator's message and all the work it
/// sets off, from `user.message` to `session.idle`. Copilot's `assistant.turn_start` and `assistant.turn_end`
/// bracket each model call inside it (one per round of tool calls), so a round's end is not the turn's end; and a
/// sub-agent's events (`agentId` set) are its own rounds, its own prompt and its own model, not the session's.
pub fn map_kind(raw: &RawEvent) -> EventKind {
    let d = &raw.data;
    let sub_agent = raw.agent_id.is_some();
    match raw.event_type.as_str() {
        "user.message" | "assistant.turn_start" | "assistant.turn_end" | "session.model_change" if sub_agent => {
            EventKind::Other { name: raw.event_type.clone() }
        }
        "user.message" => EventKind::UserMessage {
            text: s(d, "content"),
        },
        "assistant.turn_start" => EventKind::TurnStarted,
        // One model call ended; the turn goes on until `session.idle` (or `abort`).
        "assistant.turn_end" => EventKind::Other { name: raw.event_type.clone() },
        "assistant.message_delta" => EventKind::AssistantDelta {
            message_id: s(d, "messageId"),
            text: s(d, "deltaContent"),
        },
        "assistant.message" => EventKind::AssistantMessage {
            message_id: s(d, "messageId"),
            text: s(d, "content"),
        },
        "assistant.reasoning_delta" => EventKind::ReasoningDelta {
            message_id: s(d, "reasoningId"),
            text: s(d, "deltaContent"),
        },
        "assistant.reasoning" => EventKind::Reasoning {
            message_id: s(d, "reasoningId"),
            text: s(d, "content"),
        },
        "tool.execution_start" => EventKind::ToolStarted {
            call_id: s(d, "toolCallId"),
            name: s(d, "toolName"),
            summary: tool_summary(d.get("arguments")),
            diff: d.get("arguments").and_then(radar_sessions::event::edit_diff),
        },
        "tool.execution_complete" => {
            let ok = d.get("success").and_then(Value::as_bool).unwrap_or(false);
            let summary = if ok {
                cut(
                    d.pointer("/result/content")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    160,
                )
            } else {
                cut(
                    d.pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("failed"),
                    160,
                )
            };
            EventKind::ToolCompleted {
                call_id: s(d, "toolCallId"),
                ok,
                summary,
            }
        }
        // A request a hook already answered (`resolvedByHook`) is not the operator's: the SDK
        // does not call radar's handler for it, so the dock must not wait on it.
        "permission.requested" if d.get("resolvedByHook").and_then(Value::as_bool) == Some(true) => {
            EventKind::Other {
                name: raw.event_type.clone(),
            }
        }
        "permission.requested" => match permission_request(d) {
            Some(request) => EventKind::PermissionRequested { request },
            None => EventKind::Other {
                name: raw.event_type.clone(),
            },
        },
        // `approved`, `approved-for-session`, `approved-for-location`; everything else refused it.
        "permission.completed" => EventKind::PermissionResolved {
            request_id: s(d, "requestId"),
            approved: d
                .pointer("/result/kind")
                .and_then(Value::as_str)
                .is_some_and(|k| k == "approved" || k.starts_with("approved-for-")),
        },
        "user_input.requested" => EventKind::QuestionAsked {
            request: question_request(d),
        },
        // No answer is a question left unanswered (radar's decline answers "no response").
        "user_input.completed" => EventKind::QuestionResolved {
            request_id: s(d, "requestId"),
            replies: d.get("answer").and_then(Value::as_str).map(|a| {
                vec![QuestionReply {
                    question_id: QUESTION.into(),
                    values: vec![a.to_string()],
                    own_words: d.get("wasFreeform").and_then(Value::as_bool).unwrap_or(false),
                }]
            }),
        },
        "abort" => EventKind::Aborted,
        "session.idle" => EventKind::Idle,
        "session.error" => EventKind::Error {
            message: s(d, "message"),
            retryable: false,
        },
        "session.warning" | "session.info" => EventKind::Notice {
            text: s(d, "message"),
        },
        "assistant.usage" => EventKind::Usage {
            input_tokens: d.get("inputTokens").and_then(Value::as_u64),
            output_tokens: d.get("outputTokens").and_then(Value::as_u64),
        },
        "session.model_change" => EventKind::ModelChanged {
            model: s(d, "newModel"),
        },
        other => EventKind::Other {
            name: other.to_string(),
        },
    }
}

/// Copilot asks one question per request; this is its key within it.
pub const QUESTION: &str = "0";

/// The question in a `user_input.requested` event's data (the SDK's
/// `UserInputRequestedData`: requestId, question, choices, allowFreeform, toolCallId).
/// With no choices, only typed words can answer it.
pub fn question_request(d: &Value) -> QuestionRequest {
    let choices: Vec<QuestionChoice> = d
        .get("choices")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|label| QuestionChoice { label: label.to_string(), description: None })
                .collect()
        })
        .unwrap_or_default();
    let free_text = choices.is_empty() || d.get("allowFreeform").and_then(Value::as_bool).unwrap_or(true);
    QuestionRequest {
        request_id: s(d, "requestId"),
        questions: vec![Question {
            id: QUESTION.into(),
            header: None,
            text: s(d, "question"),
            choices,
            free_text,
            multiple: false,
            secret: false,
        }],
        tool_call_id: d.get("toolCallId").and_then(Value::as_str).map(str::to_string),
    }
}

/// One line for a tool call: the shell command, else its arguments, bounded.
fn tool_summary(args: Option<&Value>) -> String {
    let Some(args) = args else {
        return String::new();
    };
    for key in ["command", "path", "url", "description"] {
        if let Some(t) = args.get(key).and_then(Value::as_str) {
            return cut(t, 160);
        }
    }
    cut(&args.to_string(), 160)
}

/// Copilot's own "approve for session" for a request (the `permission.requested` data, or
/// the SDK handler's data): the decision `handlePendingPermissionRequest` takes, with what
/// the CLI remembers for the rest of the session (the commands' identifiers, writes, reads,
/// the MCP tool, the URL's domain). `None` for a kind it has no session approval for.
pub fn session_decision(data: &Value) -> Option<Value> {
    let r = data.get("permissionRequest")?;
    let prompt = data.get("promptRequest");
    let approval = |a: Value| Some(serde_json::json!({ "kind": "approve-for-session", "approval": a }));
    match r.get("kind").and_then(Value::as_str)? {
        "shell" => {
            let ids: Vec<String> = prompt
                .and_then(|p| p.get("commandIdentifiers"))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .filter(|v: &Vec<String>| !v.is_empty())
                .or_else(|| {
                    r.get("commands").and_then(Value::as_array).map(|a| {
                        a.iter().map(|c| s(c, "identifier")).filter(|i| !i.is_empty()).collect()
                    })
                })
                .unwrap_or_default();
            if ids.is_empty() {
                return None;
            }
            approval(serde_json::json!({ "kind": "commands", "commandIdentifiers": ids }))
        }
        "write" => approval(serde_json::json!({ "kind": "write" })),
        "read" => approval(serde_json::json!({ "kind": "read" })),
        "memory" => approval(serde_json::json!({ "kind": "memory" })),
        "mcp" => approval(serde_json::json!({ "kind": "mcp", "serverName": s(r, "serverName"), "toolName": s(r, "toolName") })),
        "custom-tool" => approval(serde_json::json!({ "kind": "custom-tool", "toolName": s(r, "toolName") })),
        "url" => {
            let url = s(r, "url");
            let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
            let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
            let domain = host.rsplit_once('@').map_or(host, |(_, h)| h).split(':').next().unwrap_or_default();
            (!domain.is_empty()).then(|| serde_json::json!({ "kind": "approve-for-session", "domain": domain }))
        }
        _ => None,
    }
}

/// The permission request in a `permission.requested` event's data (or the SDK
/// handler's data, which has the same shape).
pub fn permission_request(data: &Value) -> Option<PermissionRequest> {
    let request_id = data.get("requestId").and_then(Value::as_str)?.to_string();
    let r = data.get("permissionRequest")?;
    let kind_s = r.get("kind").and_then(Value::as_str).unwrap_or("other");
    let (kind, target) = match kind_s {
        "shell" => (
            PermissionKind::Shell,
            PermissionTarget::Command {
                command: s(r, "fullCommandText"),
                segments: r
                    .get("commandSegments")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|c| s(c, "fullCommandText")).collect())
                    .unwrap_or_default(),
            },
        ),
        "write" => (
            PermissionKind::Write,
            PermissionTarget::Write {
                path: s(r, "fileName"),
                diff: r.get("diff").and_then(Value::as_str).map(str::to_string),
            },
        ),
        "read" => (
            PermissionKind::Read,
            PermissionTarget::Path { path: s(r, "path") },
        ),
        "url" => (
            PermissionKind::Url,
            PermissionTarget::Url { url: s(r, "url") },
        ),
        "mcp" => (
            PermissionKind::Mcp,
            PermissionTarget::Mcp {
                server: s(r, "serverName"),
                tool: s(r, "toolName"),
                arguments: r.get("args").map(Value::to_string),
            },
        ),
        "custom-tool" => (
            PermissionKind::Other,
            PermissionTarget::Other {
                description: format!(
                    "{}: {}",
                    s(r, "toolName"),
                    r.get("args").map(Value::to_string).unwrap_or_default()
                ),
            },
        ),
        other => {
            let why = [s(r, "intention"), s(r, "toolName"), s(r, "hookMessage")]
                .into_iter()
                .find(|t| !t.is_empty())
                .unwrap_or_else(|| other.to_string());
            (
                PermissionKind::Other,
                PermissionTarget::Other {
                    description: format!("{other}: {why}"),
                },
            )
        }
    };
    Some(PermissionRequest {
        request_id,
        kind,
        target,
        tool_call_id: r
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_string),
        can_offer_session_approval: r
            .get("canOfferSessionApproval")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(t: &str, ephemeral: bool, data: Value) -> RawEvent {
        RawEvent {
            id: "e1".into(),
            timestamp: String::new(),
            parent_id: None,
            ephemeral,
            agent_id: None,
            event_type: t.into(),
            data,
        }
    }

    #[test]
    fn ask_user_is_a_question_and_its_completion_says_what_was_answered() {
        let d = json!({"requestId":"q1","question":"Which?","choices":["a","b"],"allowFreeform":false,"toolCallId":"c9"});
        let EventKind::QuestionAsked { request } = map_kind(&raw("user_input.requested", false, d)) else {
            panic!("not a question")
        };
        assert_eq!(request.request_id, "q1");
        assert_eq!(request.tool_call_id.as_deref(), Some("c9"));
        let q = &request.questions[0];
        assert_eq!((q.id.as_str(), q.text.as_str(), q.free_text), (QUESTION, "Which?", false));
        assert_eq!(q.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        // With no choices, only typed words answer it.
        let EventKind::QuestionAsked { request } = map_kind(&raw("user_input.requested", false, json!({"requestId":"q2","question":"Name?"}))) else {
            panic!("not a question")
        };
        assert!(request.questions[0].free_text);
        assert_eq!(
            map_kind(&raw("user_input.completed", false, json!({"requestId":"q1","answer":"b","wasFreeform":false}))),
            EventKind::QuestionResolved {
                request_id: "q1".into(),
                replies: Some(vec![QuestionReply { question_id: QUESTION.into(), values: vec!["b".into()], own_words: false }]),
            }
        );
        // No answer: radar declined it (no response).
        assert_eq!(
            map_kind(&raw("user_input.completed", false, json!({"requestId":"q1"}))),
            EventKind::QuestionResolved { request_id: "q1".into(), replies: None }
        );
    }

    #[test]
    fn a_shell_request_keeps_the_exact_command() {
        let d = json!({"requestId":"r1","permissionRequest":{"kind":"shell","fullCommandText":"ls -la","commandSegments":[{"fullCommandText":"ls -la","identifier":"ls"}],"canOfferSessionApproval":true,"toolCallId":"c1"}});
        let EventKind::PermissionRequested { request } =
            map_kind(&raw("permission.requested", false, d))
        else {
            panic!()
        };
        assert_eq!(request.request_id, "r1");
        assert_eq!(request.kind, PermissionKind::Shell);
        assert_eq!(
            request.target,
            PermissionTarget::Command {
                command: "ls -la".into(),
                segments: vec!["ls -la".into()]
            }
        );
        assert!(request.can_offer_session_approval);
    }

    #[test]
    fn approve_for_session_is_copilots_own_decision_for_the_kind_of_request() {
        let shell = json!({"requestId":"r1","permissionRequest":{"kind":"shell","commands":[{"identifier":"ls","readOnly":false}]},"promptRequest":{"kind":"commands","commandIdentifiers":["ls","git status"]}});
        assert_eq!(
            session_decision(&shell),
            Some(json!({"kind":"approve-for-session","approval":{"kind":"commands","commandIdentifiers":["ls","git status"]}}))
        );
        // Without a prompt request, the parsed commands' identifiers.
        let bare = json!({"permissionRequest":{"kind":"shell","commands":[{"identifier":"cargo","readOnly":false}]}});
        assert_eq!(session_decision(&bare).unwrap()["approval"]["commandIdentifiers"], json!(["cargo"]));
        assert_eq!(session_decision(&json!({"permissionRequest":{"kind":"shell","commands":[]}})), None);
        assert_eq!(session_decision(&json!({"permissionRequest":{"kind":"write"}})).unwrap()["approval"], json!({"kind":"write"}));
        assert_eq!(
            session_decision(&json!({"permissionRequest":{"kind":"mcp","serverName":"gh","toolName":"search"}})).unwrap()["approval"],
            json!({"kind":"mcp","serverName":"gh","toolName":"search"})
        );
        assert_eq!(
            session_decision(&json!({"permissionRequest":{"kind":"url","url":"https://user@docs.rs:443/a?b"}})),
            Some(json!({"kind":"approve-for-session","domain":"docs.rs"}))
        );
        assert_eq!(session_decision(&json!({"permissionRequest":{"kind":"hook"}})), None);
        // The SDK takes it as its approve-for-session decision.
        let d: github_copilot_sdk::PermissionDecision = serde_json::from_value(session_decision(&shell).unwrap()).unwrap();
        assert!(matches!(d, github_copilot_sdk::PermissionDecision::ApproveForSession(_)), "{d:?}");
    }

    #[test]
    fn a_request_a_hook_resolved_does_not_wait_and_a_session_approval_is_an_approval() {
        let d = json!({"requestId":"r1","resolvedByHook":true,"permissionRequest":{"kind":"shell","fullCommandText":"ls"}});
        assert!(matches!(map_kind(&raw("permission.requested", false, d)), EventKind::Other { .. }));
        for (kind, approved) in [("approved", true), ("approved-for-session", true), ("approved-for-location", true), ("denied-by-rules", false)] {
            assert_eq!(
                map_kind(&raw("permission.completed", false, json!({"requestId":"r1","result":{"kind":kind}}))),
                EventKind::PermissionResolved { request_id: "r1".into(), approved },
                "{kind}"
            );
        }
    }

    #[test]
    fn a_write_request_carries_the_diff() {
        let d = json!({"requestId":"r2","permissionRequest":{"kind":"write","fileName":"/x/notes.txt","diff":"+hi"}});
        let EventKind::PermissionRequested { request } =
            map_kind(&raw("permission.requested", false, d))
        else {
            panic!()
        };
        assert_eq!(
            request.target,
            PermissionTarget::Write {
                path: "/x/notes.txt".into(),
                diff: Some("+hi".into())
            }
        );
    }

    #[test]
    fn deltas_and_messages_use_the_message_id() {
        assert_eq!(
            map_kind(&raw(
                "assistant.message_delta",
                true,
                json!({"messageId":"m","deltaContent":"Hi"})
            )),
            EventKind::AssistantDelta {
                message_id: "m".into(),
                text: "Hi".into()
            }
        );
        assert_eq!(
            map_kind(&raw(
                "assistant.message",
                false,
                json!({"messageId":"m","content":"Hi there"})
            )),
            EventKind::AssistantMessage {
                message_id: "m".into(),
                text: "Hi there".into()
            }
        );
    }

    #[test]
    fn a_denied_tool_is_a_failed_completion() {
        let k = map_kind(&raw(
            "tool.execution_complete",
            false,
            json!({"toolCallId":"c","success":false,"error":{"code":"denied","message":"The user rejected this tool call."}}),
        ));
        assert_eq!(
            k,
            EventKind::ToolCompleted {
                call_id: "c".into(),
                ok: false,
                summary: "The user rejected this tool call.".into()
            }
        );
    }

    /// The order 1.0.97 gives a review that hands work to a sub-agent and answers in a second round (recorded
    /// live 2026-10-14, event types and agent ids only).
    #[test]
    fn only_session_idle_ends_the_turn_and_a_sub_agents_rounds_prompt_and_model_are_not_the_sessions() {
        let main = |t: &str| raw(t, false, json!({}));
        let sub = |t: &str, d: Value| RawEvent { agent_id: Some("db02".into()), ..raw(t, false, d) };
        let ended = |k: &EventKind| matches!(k, EventKind::TurnEnded | EventKind::Idle);
        let stream = [
            main("assistant.turn_start"),
            raw("tool.execution_start", false, json!({"toolCallId":"c1","toolName":"task","arguments":{"agent_type":"code-review"}})),
            sub("user.message", json!({"content":"Review the repository"})),
            sub("session.model_change", json!({"newModel":"gpt-5.6-luna"})),
            sub("assistant.turn_start", json!({})),
            sub("assistant.turn_end", json!({})),
            raw("tool.execution_complete", false, json!({"toolCallId":"c1","success":true,"result":{"content":"## Issue"}})),
            main("assistant.turn_end"),
            main("assistant.turn_start"),
            raw("assistant.message", false, json!({"messageId":"m","content":"**Verdict**: request changes"})),
            main("assistant.turn_end"),
        ];
        let kinds: Vec<EventKind> = stream.iter().map(map_kind).collect();
        assert!(!kinds.iter().any(ended), "{kinds:?}");
        assert!(!kinds.iter().any(|k| matches!(k, EventKind::UserMessage { .. } | EventKind::ModelChanged { .. })), "{kinds:?}");
        assert_eq!(kinds.iter().filter(|k| **k == EventKind::TurnStarted).count(), 2, "the session's own rounds only");
        assert_eq!(map_kind(&raw("session.idle", true, json!({"mode":"interactive"}))), EventKind::Idle);
        assert_eq!(map_kind(&raw("abort", false, json!({}))), EventKind::Aborted);
        // The operator's own message is still theirs.
        assert_eq!(map_kind(&raw("user.message", false, json!({"content":"and?"}))), EventKind::UserMessage { text: "and?".into() });
    }

    #[test]
    fn unknown_events_keep_their_name_and_durability() {
        let e = raw("model.call_start", true, json!({}));
        assert_eq!(
            map_kind(&e),
            EventKind::Other {
                name: "model.call_start".into()
            }
        );
        assert_eq!(e.durability(), Durability::Ephemeral);
    }
}
