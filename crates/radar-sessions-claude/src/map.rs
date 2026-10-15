//! Sidecar event lines (sidecars/claude-agent/src/map.js) to radar's event envelope.

use radar_sessions::{
    Durability, Event, EventKind, RadarSessionId, PermissionKind, PermissionRequest, PermissionTarget,
    ProviderEventId, ProviderId, ProviderSessionId, Question, QuestionChoice, QuestionReply,
    QuestionRequest, SlashCommand,
};
use serde_json::{Value, json};

/// What the adapter remembers per session to number and interpret its events.
#[derive(Debug, Default)]
pub struct SessionState {
    pub provider_session_id: Option<ProviderSessionId>,
    /// The sidecar numbers each session's lines from 1 in every process it starts; `base` is
    /// what came before this process, so radar's sequence keeps rising across restarts.
    pub base: u64,
    /// Highest `base + seq` seen.
    pub max: u64,
    /// The sidecar holds this session now (false after the connection dropped).
    pub live: bool,
    /// `stop()` was called and the turn has not ended yet: the error that ends it is an abort.
    pub stopping: bool,
    pub cwd: Option<String>,
}

impl SessionState {
    /// A new process now serves this session: its numbering starts again from 1.
    pub fn restart_numbering(&mut self) {
        self.base = self.max;
    }
}

/// Envelope sequences leave room for the events one sidecar line can turn into.
const SUB: u64 = 4;

pub fn provider() -> ProviderId {
    ProviderId::new("claude")
}

/// Bounded text for a row summary.
pub fn clip(s: &str, max: usize) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let mut t: String = one_line.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn tool_summary(name: &str, input: &Value) -> String {
    let field = |k: &str| input.get(k).and_then(Value::as_str);
    let s = match name {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" => field("file_path"),
        "NotebookEdit" => field("notebook_path"),
        "Glob" | "Grep" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "AskUserQuestion" => input.pointer("/questions/0/question").and_then(Value::as_str),
        _ => None,
    };
    clip(&s.map_or_else(|| input.to_string(), str::to_string), 200)
}

fn target(p: &Value) -> (PermissionKind, PermissionTarget) {
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    match p["kind"].as_str().unwrap_or("other") {
        "shell" => {
            let command = s("command");
            (PermissionKind::Shell, PermissionTarget::Command { segments: vec![command.clone()], command })
        }
        "read" => (PermissionKind::Read, PermissionTarget::Path { path: s("path") }),
        "write" => {
            let diff = p.get("diff").and_then(Value::as_str).map(str::to_string);
            (PermissionKind::Write, PermissionTarget::Write { path: s("path"), diff })
        }
        "url" => (PermissionKind::Url, PermissionTarget::Url { url: s("url") }),
        "mcp" => {
            // mcp__<server>__<tool>
            let name = s("tool_name");
            let mut parts = name.splitn(3, "__");
            let (_, server, tool) = (parts.next(), parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
            (
                PermissionKind::Mcp,
                PermissionTarget::Mcp {
                    server: server.to_string(),
                    tool: tool.to_string(),
                    arguments: p.get("input").map(|i| clip(&i.to_string(), 500)),
                },
            )
        }
        _ => {
            let input = p.get("input").map(Value::to_string).unwrap_or_default();
            (PermissionKind::Other, PermissionTarget::Other { description: clip(&format!("{} {input}", s("tool_name")), 300) })
        }
    }
}

/// The slash commands in a sidecar `{"type":"commands"}` line (from `Query.supportedCommands()`):
/// each is run by sending `/name args` as a prompt. Rows without a name are dropped.
pub fn commands(v: &Value) -> Vec<SlashCommand> {
    v["commands"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let name = c["name"].as_str().filter(|n| !n.is_empty())?;
                    let mut cmd = SlashCommand::new(name, clip(c["description"].as_str().unwrap_or_default(), 200));
                    if let Some(hint) = c["argument_hint"].as_str() {
                        cmd = cmd.with_hint(hint);
                    }
                    Some(cmd)
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct Mapper<'a> {
    pub radar: &'a RadarSessionId,
    pub state: &'a mut SessionState,
    pub now: i64,
    /// Events already made from the current line.
    pub sub: u64,
    pub line_seq: u64,
}

impl<'a> Mapper<'a> {
    pub fn new(radar: &'a RadarSessionId, state: &'a mut SessionState, now: i64) -> Self {
        Self { radar, state, now, sub: 0, line_seq: 0 }
    }

    fn event(&mut self, id: Option<&str>, durability: Durability, kind: EventKind, parent: Option<String>, payload: Value) -> Event {
        let sequence = self.line_seq * SUB + self.sub;
        self.sub += 1;
        Event {
            radar_session_id: self.radar.clone(),
            provider: provider(),
            provider_session_id: self.state.provider_session_id.clone().unwrap_or_else(|| ProviderSessionId::new("")),
            provider_event_id: id.map(ProviderEventId::new),
            sequence,
            observed_at: self.now,
            kind,
            durability,
            agent: None,
            parent: parent.map(ProviderEventId::new),
            payload,
        }
    }

    /// One sidecar `{"type":"event",..}` line to zero or more envelopes, in order.
    pub fn map(&mut self, v: &Value) -> Vec<Event> {
        if let Some(p) = v["provider_session_id"].as_str().filter(|p| !p.is_empty()) {
            self.state.provider_session_id = Some(ProviderSessionId::new(p));
        }
        let n = v["seq"].as_u64().map_or(self.state.max + 1, |s| self.state.base + s);
        self.state.max = self.state.max.max(n);
        self.line_seq = n;
        self.sub = 0;
        let id = v["provider_event_id"].as_str();
        let durability = if v["durability"] == "ephemeral" { Durability::Ephemeral } else { Durability::Persisted };
        let p = &v["payload"];
        let text = |k: &str| p[k].as_str().unwrap_or_default().to_string();
        let message_id = || p["message_id"].as_str().map(str::to_string).unwrap_or_else(|| id.unwrap_or_default().to_string());
        let mut out = vec![];
        match v["kind"].as_str().unwrap_or_default() {
            "session_started" => {
                let model = text("model");
                if !model.is_empty() {
                    out.push(self.event(id, durability, EventKind::ModelChanged { model }, None, json!({ "permission_mode": p["permission_mode"] })));
                }
            }
            "turn_started" => out.push(self.event(None, Durability::Ephemeral, EventKind::TurnStarted, None, Value::Null)),
            "user_message" => out.push(self.event(id, durability, EventKind::UserMessage { text: text("text") }, None, Value::Null)),
            "text_delta" => out.push(self.event(id, durability, EventKind::AssistantDelta { message_id: message_id(), text: text("text") }, None, Value::Null)),
            "reasoning_delta" => out.push(self.event(id, durability, EventKind::ReasoningDelta { message_id: message_id(), text: text("text") }, None, Value::Null)),
            "assistant_message" => {
                let payload = json!({ "model": p["model"] });
                out.push(self.event(id, durability, EventKind::AssistantMessage { message_id: message_id(), text: text("text") }, None, payload));
            }
            "reasoning" => out.push(self.event(id, durability, EventKind::Reasoning { message_id: message_id(), text: text("text") }, None, Value::Null)),
            "tool_call" => {
                let name = text("name");
                let summary = tool_summary(&name, &p["input"]);
                let diff = radar_sessions::event::edit_diff(&p["input"]);
                out.push(self.event(id, durability, EventKind::ToolStarted { call_id: text("tool_use_id"), name, summary, diff }, None, Value::Null));
            }
            "tool_result" => {
                let call_id = text("tool_use_id");
                let parent = Some(format!("tool:{call_id}"));
                let ok = p["is_error"] != true;
                out.push(self.event(id, durability, EventKind::ToolCompleted { call_id, ok, summary: clip(&text("content"), 200) }, parent, Value::Null));
            }
            "permission_request" => {
                let (kind, target) = target(p);
                let request = PermissionRequest {
                    request_id: text("request_id"),
                    kind,
                    target,
                    tool_call_id: p["tool_use_id"].as_str().map(str::to_string),
                    // Approve for session: Claude Code's suggested rules with destination
                    // `session` where it gives some, and radar remembers the request either way.
                    can_offer_session_approval: true,
                };
                out.push(self.event(id, durability, EventKind::PermissionRequested { request }, None, Value::Null));
            }
            "question_request" => {
                // AskUserQuestion: each question is keyed by its text (the answers go back by it);
                // Claude Code always lets the operator type their own answer ("Other").
                let questions = p["questions"]
                    .as_array()
                    .map(|qs| {
                        qs.iter()
                            .map(|q| {
                                let text = q["question"].as_str().unwrap_or_default().to_string();
                                Question {
                                    id: text.clone(),
                                    header: q["header"].as_str().filter(|h| !h.is_empty()).map(str::to_string),
                                    text,
                                    choices: q["options"]
                                        .as_array()
                                        .map(|os| {
                                            os.iter()
                                                .map(|o| QuestionChoice {
                                                    label: o["label"].as_str().unwrap_or_default().to_string(),
                                                    description: o["description"].as_str().filter(|d| !d.is_empty()).map(str::to_string),
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default(),
                                    free_text: true,
                                    multiple: q["multi_select"] == true,
                                    secret: false,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let request = QuestionRequest { request_id: text("request_id"), questions, tool_call_id: p["tool_use_id"].as_str().map(str::to_string) };
                out.push(self.event(id, durability, EventKind::QuestionAsked { request }, None, Value::Null));
            }
            "question_resolved" => {
                // answers: question text -> answer, or null when declined.
                let replies = p["answers"].as_object().map(|a| {
                    a.iter()
                        .map(|(q, v)| QuestionReply { question_id: q.clone(), values: vec![v.as_str().map_or_else(|| v.to_string(), str::to_string)], own_words: false })
                        .collect()
                });
                out.push(self.event(id, durability, EventKind::QuestionResolved { request_id: text("request_id"), replies }, None, Value::Null));
            }
            "permission_resolved" => {
                let approved = p["behavior"] == "allow";
                out.push(self.event(id, durability, EventKind::PermissionResolved { request_id: text("request_id"), approved }, None, Value::Null));
            }
            "turn_end" => {
                let failed = p["is_error"] == true;
                if failed && self.state.stopping {
                    out.push(self.event(id, durability, EventKind::Aborted, None, Value::Null));
                } else if failed {
                    let msg = p["errors"].as_array().and_then(|e| e.first()).and_then(Value::as_str).or(p["result"].as_str()).unwrap_or("the turn failed");
                    out.push(self.event(id, durability, EventKind::Error { message: clip(msg, 300), retryable: false }, None, Value::Null));
                } else {
                    out.push(self.event(id, durability, EventKind::TurnEnded, None, json!({ "cost_usd": p["total_cost_usd"] })));
                }
                self.state.stopping = false;
                if p["usage"].is_object() {
                    let u = |k: &str| p["usage"][k].as_u64();
                    out.push(self.event(None, Durability::Ephemeral, EventKind::Usage { input_tokens: u("input_tokens"), output_tokens: u("output_tokens") }, None, Value::Null));
                }
                out.push(self.event(None, Durability::Ephemeral, EventKind::Idle, None, Value::Null));
            }
            "error" => out.push(self.event(None, Durability::Persisted, EventKind::Error { message: text("message"), retryable: false }, None, Value::Null)),
            "session_ended" => out.push(self.event(None, Durability::Ephemeral, EventKind::Notice { text: "The Claude Code session ended.".into() }, None, Value::Null)),
            other => out.push(self.event(id, durability, EventKind::Other { name: other.to_string() }, None, Value::Null)),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_line_becomes_slash_commands_with_their_hint_and_description() {
        let v = json!({"type": "commands", "session": "g1", "commands": [
            {"name": "compact", "description": "Free up context by summarising the conversation", "argument_hint": "[instructions]"},
            {"name": "context", "description": "Show context usage"},
            {"name": "", "description": "nameless"},
            {"name": "loop", "description": "Run a prompt\n  on an interval", "argument_hint": ""},
        ]});
        assert_eq!(
            commands(&v),
            vec![
                SlashCommand::new("compact", "Free up context by summarising the conversation").with_hint("[instructions]"),
                SlashCommand::new("context", "Show context usage"),
                SlashCommand::new("loop", "Run a prompt on an interval"),
            ]
        );
        assert!(commands(&json!({"type": "commands"})).is_empty());
    }
}
