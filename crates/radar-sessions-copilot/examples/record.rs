//! Records one live Copilot stream for the adapter's replay tests.
//!
//! cargo run -p radar-sessions-copilot --example record -- <scratch-repo> <out.jsonl>
//!
//! Uses the installed `copilot` and its real sign-in, one scratch git directory and
//! tiny prompts: turn 1 runs `ls` (approved once) and tries to write a file (denied);
//! turn 2 starts `sleep` and is aborted mid-tool; then the session is closed, resumed
//! (its persisted history is recorded) and asked one question. The session is deleted
//! with Copilot's own delete. Before committing the output run
//! `scripts/scan-secrets`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use radar_sessions::{BoxFuture, RadarSessionId, PermissionDecision, QuestionDecision};
use radar_sessions_copilot::backend::{Backend, Sink};
use radar_sessions_copilot::map::RawEvent;
use radar_sessions_copilot::SdkBackend;
use tokio::sync::Notify;

struct Recorder {
    phase: Mutex<String>,
    lines: Mutex<Vec<(String, RawEvent)>>,
    idle: Notify,
    tool_started: Notify,
}

impl Sink for Recorder {
    fn event(&self, event: RawEvent) {
        // The system prompt and model-call internals carry the environment and are not
        // what the adapter maps; leave them out of the fixture.
        let skip = event.event_type.starts_with("model.")
            || event.event_type.starts_with("sandbox.")
            || event.event_type == "system.message"
            || event.event_type == "prompt_cache_break";
        match event.event_type.as_str() {
            "session.idle" => self.idle.notify_one(),
            "tool.execution_start" => self.tool_started.notify_one(),
            _ => {}
        }
        if !skip {
            let phase = self.phase.lock().unwrap().clone();
            self.lines.lock().unwrap().push((phase, event));
        }
    }

    fn permission(
        &self,
        request_id: String,
        data: serde_json::Value,
    ) -> BoxFuture<'_, PermissionDecision> {
        let kind = data
            .pointer("/permissionRequest/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        eprintln!("permission {request_id}: {kind}");
        Box::pin(async move {
            if kind == "write" {
                PermissionDecision::Deny {
                    reason: Some("radar denies writes while recording".into()),
                }
            } else {
                PermissionDecision::ApproveOnce
            }
        })
    }

    fn question(
        &self,
        question: String,
        _choices: Vec<String>,
        _allow_freeform: bool,
    ) -> BoxFuture<'_, QuestionDecision> {
        // The recording's prompts ask nothing; a question the model asks anyway is left
        // unanswered (no response) rather than answered for the operator.
        eprintln!("question: {question}");
        Box::pin(async {
            QuestionDecision::Decline {
                reason: Some("radar answers no questions while recording".into()),
            }
        })
    }

    fn lost(&self, reason: String) {
        eprintln!("lost: {reason}");
    }

    fn restored(&self) {}
}

impl Recorder {
    /// Starts a phase and forgets the signals of the one before (a `Notify` keeps one
    /// permit, which would end the next wait at once).
    async fn set_phase(&self, p: &str) {
        *self.phase.lock().unwrap() = p.to_string();
        for n in [&self.idle, &self.tool_started] {
            while tokio::time::timeout(std::time::Duration::from_millis(1), n.notified())
                .await
                .is_ok()
            {}
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let repo = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let backend = SdkBackend::new(None);
    let rec = Arc::new(Recorder {
        phase: Mutex::new("turn1".into()),
        lines: Mutex::default(),
        idle: Notify::new(),
        tool_started: Notify::new(),
    });
    let id = RadarSessionId::new(uuid::Uuid::new_v4().to_string());
    let t = std::time::Duration::from_secs(180);

    backend.create(&id, &repo, None, &Default::default(), rec.clone()).await?;
    backend
        .send(&id, "Run the shell command `ls` in this directory and tell me the file names. Then create a file named notes.txt containing 'hi'. Keep the answer to one line.")
        .await?;
    tokio::time::timeout(t, rec.idle.notified()).await?;

    rec.set_phase("turn2").await;
    backend
        .send(
            &id,
            "Run `sleep 30; echo done` in the shell, then summarise.",
        )
        .await?;
    tokio::time::timeout(t, rec.tool_started.notified()).await?;
    rec.set_phase("turn2-after-abort").await;
    backend.abort(&id).await?;
    tokio::time::timeout(t, rec.idle.notified()).await?;

    // Resume: the persisted history is delivered first, then a live turn.
    rec.set_phase("history").await;
    backend.resume(&id, &repo, &Default::default(), rec.clone()).await?;
    rec.set_phase("turn3").await;
    backend
        .send(&id, "In one line: what file names did `ls` show earlier?")
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
    let text = text
        .replace(&repo.display().to_string(), "/work/repo")
        .replace(id.as_str(), "00000000-0000-4000-8000-000000000001")
        .replace(&home, "/home/user");
    std::fs::write(&out, text)?;
    eprintln!(
        "recorded {} events to {}",
        rec.lines.lock().unwrap().len(),
        out.display()
    );
    Ok(())
}
