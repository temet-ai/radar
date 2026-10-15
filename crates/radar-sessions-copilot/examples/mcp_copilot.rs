//! A live check: a Copilot session set up with radar's
//! own MCP server calls its `echo` tool, on create and again after a resume.
//!
//!   cargo run -p radar-sessions --example mcp_listen -- <sock> <radar> <server.json> &
//!   cargo run -p radar-sessions-copilot --example mcp_copilot --<scratch-repo> <out.jsonl> <server.json>
//!
//! The installed `copilot` with its real sign-in, through radar's own SDK backend (so the setup
//! goes through `setup::apply_create` and `apply_resume`). Every permission request is printed;
//! one for radar's server (kind `mcp`, server `radar`) is approved once, anything else denied. The
//! session is deleted with Copilot's own delete. Scan the output with
//! `scripts/scan-secrets` before committing it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use radar_sessions::adapter::SessionSetup;
use radar_sessions::{BoxFuture, RadarSessionId, PermissionDecision, QuestionDecision, ToolServer};
use radar_sessions_copilot::SdkBackend;
use radar_sessions_copilot::backend::{Backend, Sink};
use radar_sessions_copilot::map::RawEvent;
use tokio::sync::Notify;

struct Recorder {
    phase: Mutex<String>,
    lines: Mutex<Vec<(String, RawEvent)>>,
    asked: Mutex<Vec<serde_json::Value>>,
    idle: Notify,
}

impl Sink for Recorder {
    fn event(&self, event: RawEvent) {
        // The system prompt and model-call internals carry the environment and are not what
        // the adapter maps.
        let skip = event.event_type.starts_with("model.")
            || event.event_type.starts_with("sandbox.")
            || event.event_type == "system.message"
            || event.event_type == "prompt_cache_break";
        if event.event_type == "session.idle" {
            self.idle.notify_one();
        }
        if !skip {
            let phase = self.phase.lock().unwrap().clone();
            self.lines.lock().unwrap().push((phase, event));
        }
    }

    fn permission(&self, request_id: String, data: serde_json::Value) -> BoxFuture<'_, PermissionDecision> {
        let req = data.get("permissionRequest").cloned().unwrap_or(data);
        let kind = req.get("kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let server = req.get("serverName").and_then(|v| v.as_str()).unwrap_or("").to_string();
        eprintln!("permission {request_id}: kind={kind} server={server} tool={:?}", req.get("toolName"));
        self.asked.lock().unwrap().push(req);
        Box::pin(async move {
            if kind == "mcp" && server == "radar" {
                PermissionDecision::ApproveOnce
            } else {
                PermissionDecision::Deny { reason: Some("only radar's echo is approved in this check".into()) }
            }
        })
    }

    fn question(&self, question: String, _choices: Vec<String>, _allow_freeform: bool) -> BoxFuture<'_, QuestionDecision> {
        eprintln!("question: {question}");
        Box::pin(async { QuestionDecision::Decline { reason: Some("no questions in this check".into()) } })
    }

    fn lost(&self, reason: String) {
        eprintln!("lost: {reason}");
    }

    fn restored(&self) {}
}

impl Recorder {
    async fn set_phase(&self, p: &str) {
        *self.phase.lock().unwrap() = p.to_string();
        while tokio::time::timeout(std::time::Duration::from_millis(1), self.idle.notified()).await.is_ok() {}
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let repo = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let server: ToolServer = serde_json::from_str(&std::fs::read_to_string(&args[3])?)?;
    let setup = SessionSetup { tool_server: Some(server.clone()), ..Default::default() };
    let backend = SdkBackend::new(None);
    let rec = Arc::new(Recorder { phase: Mutex::new("turn1".into()), lines: Mutex::default(), asked: Mutex::default(), idle: Notify::new() });
    let id = RadarSessionId::new(uuid::Uuid::new_v4().to_string());
    let t = std::time::Duration::from_secs(180);

    backend.create(&id, &repo, None, &setup, rec.clone()).await?;
    backend
        .send(&id, "Call the echo tool from the radar server with text 'ping' and tell me the result.")
        .await?;
    tokio::time::timeout(t, rec.idle.notified()).await?;

    // Resume: the server is given again, and must be there for a second call.
    rec.set_phase("history").await;
    backend.resume(&id, &repo, &setup, rec.clone()).await?;
    rec.set_phase("turn2").await;
    backend
        .send(&id, "Call the echo tool from the radar server again, with text 'pong', and tell me the result.")
        .await?;
    tokio::time::timeout(t, rec.idle.notified()).await?;

    backend.delete(id.as_str()).await?;

    let home = std::env::var("HOME").unwrap_or_default();
    let mut text = String::new();
    for (phase, ev) in rec.lines.lock().unwrap().iter() {
        let mut v = serde_json::to_value(ev)?;
        v["phase"] = phase.clone().into();
        text.push_str(&v.to_string());
        text.push('\n');
    }
    let ticket = server.args.last().cloned().unwrap_or_default();
    let socket = server.args.get(2).cloned().unwrap_or_default();
    let text = text
        .replace(&ticket, "0123456789abcdef0123456789abcdef")
        .replace(&socket, "/run/user/1000/radar/mcp/spike.sock")
        .replace(&server.command, "/usr/local/bin/radar")
        .replace(&repo.display().to_string(), "/work/repo")
        .replace(id.as_str(), "00000000-0000-4000-8000-000000000001")
        .replace(&home, "/home/user");
    std::fs::write(&out, text)?;
    eprintln!("recorded {} events, {} permission requests, to {}", rec.lines.lock().unwrap().len(), rec.asked.lock().unwrap().len(), out.display());
    Ok(())
}
