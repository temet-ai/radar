//! The adapter against a recorded live run of the sidecar (see tests/fixtures/live.ndjson and
//! sidecars/claude-agent/scripts/record.mjs). No node, no Claude Code, no clock.

#[path = "replay/mod.rs"]
mod replay;

use std::path::PathBuf;
use std::sync::Arc;

use radar_sessions::{
    Adapter, AuthStatus, CreateSession, Durability, Event, EventKind, RadarSessionId, PermissionAnswer, PermissionDecision,
    PermissionKind, PermissionTarget, ResumeSession, SendInput, Update,
};
use radar_sessions::adapter::Updates;
use radar_sessions_claude::{ClaudeAdapter, ClaudeConfig};
use replay::Replay;

const RADAR: &str = "radar-live-1";

fn adapter(replay: Replay) -> ClaudeAdapter {
    ClaudeAdapter::new(Box::new(replay), ClaudeConfig::default(), Arc::new(|| 1_000))
}

/// Updates until `stop` matches an event; everything before it, and it, are returned.
async fn until(rx: &mut Updates, stop: impl Fn(&EventKind) -> bool) -> Vec<Event> {
    let mut out = vec![];
    while let Some(u) = rx.recv().await {
        if let Update::Event(e) = u {
            let done = stop(&e.kind);
            out.push(e);
            if done {
                return out;
            }
        }
    }
    panic!("updates ended; got {:?}", out.iter().map(|e| &e.kind).collect::<Vec<_>>());
}

fn kinds(es: &[Event]) -> Vec<&'static str> {
    es.iter()
        .map(|e| match &e.kind {
            EventKind::UserMessage { .. } => "user",
            EventKind::TurnStarted => "turn_started",
            EventKind::TurnEnded => "turn_ended",
            EventKind::AssistantDelta { .. } => "delta",
            EventKind::AssistantMessage { .. } => "message",
            EventKind::ReasoningDelta { .. } => "reasoning_delta",
            EventKind::Reasoning { .. } => "reasoning",
            EventKind::ToolStarted { .. } => "tool_started",
            EventKind::ToolCompleted { .. } => "tool_completed",
            EventKind::PermissionRequested { .. } => "permission",
            EventKind::PermissionResolved { .. } => "resolved",
            EventKind::QuestionAsked { .. } => "question",
            EventKind::QuestionResolved { .. } => "answered",
            EventKind::Aborted => "aborted",
            EventKind::Idle => "idle",
            EventKind::Error { .. } => "error",
            EventKind::Notice { .. } => "notice",
            EventKind::Usage { .. } => "usage",
            EventKind::ModelChanged { .. } => "model",
            EventKind::Other { .. } => "other",
        })
        .collect()
}

fn radar() -> RadarSessionId {
    RadarSessionId::new(RADAR)
}

fn answer(id: &str, decision: PermissionDecision) -> PermissionAnswer {
    PermissionAnswer { request_id: id.into(), decision }
}

fn permission(es: &[Event]) -> radar_sessions::PermissionRequest {
    es.iter()
        .find_map(|e| match &e.kind {
            EventKind::PermissionRequested { request } => Some(request.clone()),
            _ => None,
        })
        .expect("a permission request")
}

fn is_perm(k: &EventKind) -> bool {
    matches!(k, EventKind::PermissionRequested { .. })
}
fn is_idle(k: &EventKind) -> bool {
    matches!(k, EventKind::Idle)
}

#[tokio::test]
async fn status_says_signed_in_with_the_version_and_the_sdk() {
    let a = adapter(Replay::from(0));
    let s = a.status().await;
    assert!(s.installed);
    assert_eq!(s.version.as_deref(), Some("2.1.296"));
    assert_eq!(s.auth, AuthStatus::SignedIn { as_user: "user@example.com".into() });
    assert_eq!(s.protocol.as_deref(), Some("Agent SDK 0.3.296"));
    assert_eq!(s.error, None);
    // What it cannot do is declared, not faked.
    assert!(s.capabilities.permissions && s.capabilities.resume && s.capabilities.stop);
    // A message sent mid-turn goes into the session's streaming input: Claude reads it in that turn.
    assert!(s.capabilities.steer);
    assert!(!s.capabilities.unsupported.iter().any(|u| u.control == "steer"));
}

#[tokio::test]
async fn a_session_streams_asks_permission_and_stops_on_the_recorded_run() {
    let a = adapter(Replay::from(0));
    let mut rx = a.subscribe().await.unwrap();
    assert!(a.subscribe().await.is_err(), "a second subscribe is an error");
    a.status().await;

    let pid = a.create(CreateSession { radar_session_id: radar(), cwd: PathBuf::from("/tmp/radar-live/repo"), model: None, setup: Default::default() }).await.unwrap();
    assert!(!pid.as_str().is_empty());

    // Turn 1: `ls`, approved.
    a.send(&radar(), SendInput { text: "Run `ls`".into() }).await.unwrap();
    let es = until(&mut rx, is_perm).await;
    let req = permission(&es);
    assert_eq!(req.kind, PermissionKind::Shell);
    assert_eq!(req.target, PermissionTarget::Command { command: "ls".into(), segments: vec!["ls".into()] });
    assert!(req.can_offer_session_approval);
    a.answer_permission(&radar(), answer(&req.request_id, PermissionDecision::ApproveOnce)).await.unwrap();
    let mut es2 = until(&mut rx, is_idle).await;
    let mut all = es;
    all.append(&mut es2);
    let k = kinds(&all);
    assert_eq!(k[0], "turn_started");
    assert!(k.contains(&"user") && k.contains(&"tool_started") && k.contains(&"resolved"));
    assert!(k.contains(&"tool_completed") && k.contains(&"message") && k.contains(&"turn_ended"));
    assert!(all.iter().any(|e| matches!(&e.kind, EventKind::ToolCompleted { ok: true, summary, .. } if summary == "a.txt")));
    assert!(all.iter().any(|e| matches!(&e.kind, EventKind::AssistantMessage { text, .. } if text.contains("a.txt"))));
    // Deltas are ephemeral and the message that replaces them shares their message id.
    let delta = all.iter().find(|e| matches!(e.kind, EventKind::AssistantDelta { .. }));
    if let Some(d) = delta {
        assert_eq!(d.durability, Durability::Ephemeral);
        let (EventKind::AssistantDelta { message_id: dm, .. }, Some(m)) = (&d.kind, all.iter().find(|e| matches!(e.kind, EventKind::AssistantMessage { .. }))) else { panic!() };
        let EventKind::AssistantMessage { message_id: mm, .. } = &m.kind else { panic!() };
        assert_eq!(dm, mm);
    }
    assert_ordered(&all);

    // Turn 2: a write, denied; the model hears the reason.
    a.send(&radar(), SendInput { text: "Create b.txt".into() }).await.unwrap();
    let es = until(&mut rx, is_perm).await;
    let req = permission(&es);
    assert_eq!(req.kind, PermissionKind::Write);
    let PermissionTarget::Write { path, diff } = &req.target else { panic!("{:?}", req.target) };
    assert!(path.ends_with("b.txt"));
    assert!(diff.as_deref().is_some_and(|d| d.starts_with('+')), "a write shows what it writes: {diff:?}");
    a.answer_permission(&radar(), answer(&req.request_id, PermissionDecision::Deny { reason: Some("Denied in radar.".into()) })).await.unwrap();
    let es = until(&mut rx, is_idle).await;
    assert!(es.iter().any(|e| matches!(&e.kind, EventKind::ToolCompleted { ok: false, summary, .. } if summary == "Denied in radar.")));
    assert!(es.iter().any(|e| matches!(&e.kind, EventKind::PermissionResolved { approved: false, .. })));
    assert!(es.iter().any(|e| matches!(e.kind, EventKind::AssistantMessage { .. })), "the model answers the denial");

    // Turn 3: stopped mid-tool.
    a.send(&radar(), SendInput { text: "Run `sleep 30`".into() }).await.unwrap();
    let es = until(&mut rx, is_perm).await;
    a.answer_permission(&radar(), answer(&permission(&es).request_id, PermissionDecision::ApproveOnce)).await.unwrap();
    until(&mut rx, |k| matches!(k, EventKind::ToolStarted { .. } | EventKind::PermissionResolved { .. })).await;
    a.stop(&radar()).await.unwrap();
    let es = until(&mut rx, is_idle).await;
    assert!(es.iter().any(|e| matches!(e.kind, EventKind::Aborted)), "an interrupted turn ends as Aborted, not as an error: {:?}", kinds(&es));
    assert!(!es.iter().any(|e| matches!(e.kind, EventKind::Error { .. })));
}

fn assert_ordered(es: &[Event]) {
    for w in es.windows(2) {
        assert!(w[1].sequence > w[0].sequence, "sequence rises per session");
    }
    for e in es {
        if e.durability == Durability::Persisted && !matches!(e.kind, EventKind::Error { .. } | EventKind::Notice { .. }) {
            assert!(e.provider_event_id.is_some(), "persisted events carry the dedupe key: {:?}", e.kind);
        }
        assert_eq!(e.observed_at, 1_000);
        assert_eq!(e.provider.as_str(), "claude");
    }
}

#[tokio::test]
async fn resume_replays_the_providers_history_after_the_cursor() {
    // Process 2 of the recording: create(resume), history, a question, its answer. No cursor: all of it.
    let a = adapter(Replay::from(1));
    let mut rx = a.subscribe().await.unwrap();
    let pid = radar_sessions::ProviderSessionId::new("96ca62a0-5037-429e-a9f3-4a26bb336113");
    a.resume(ResumeSession { radar_session_id: radar(), provider_session_id: pid, cwd: PathBuf::from("/tmp/radar-live/repo"), cursor: None, setup: Default::default() }).await.unwrap();
    a.send(&radar(), SendInput { text: "What file names did you list earlier?".into() }).await.unwrap();
    let es = until(&mut rx, is_idle).await;
    assert_ordered(&es);
    // History first (what the operator typed, tool calls, answers), then the new turn.
    let k = kinds(&es);
    let first_turn = k.iter().position(|k| *k == "turn_started").unwrap();
    assert!(k[..first_turn].iter().filter(|k| **k == "user").count() >= 3, "history holds the earlier questions: {k:?}");
    assert!(es[..first_turn].iter().all(|e| e.durability == Durability::Persisted), "only persisted events come back from history");
    assert!(es[..first_turn].iter().any(|e| matches!(&e.kind, EventKind::UserMessage { text } if text.starts_with("Run `ls`"))));
}

#[test]
fn event_ids_in_history_are_the_ids_they_had_live() {
    // The store drops what it already has by provider event id, so a resumed session must not
    // show the same message twice: every persisted id seen live is in the history reply.
    let procs = replay::processes();
    let live: std::collections::HashSet<String> = procs[0]
        .iter()
        .filter(|(incoming, l)| !incoming && l["type"] == "event" && l["durability"] == "persisted")
        .filter(|(_, l)| matches!(l["kind"].as_str(), Some("user_message" | "assistant_message" | "tool_call" | "tool_result")))
        .map(|(_, l)| l["provider_event_id"].as_str().unwrap().to_string())
        .collect();
    let history: std::collections::HashSet<String> = procs[1]
        .iter()
        .filter_map(|(_, l)| l["result"]["events"].as_array())
        .flatten()
        .map(|e| e["provider_event_id"].as_str().unwrap().to_string())
        .collect();
    assert!(live.len() >= 8);
    let missing: Vec<_> = live.difference(&history).collect();
    // The synthetic "No response requested." the CLI adds after an interrupt exists only in history.
    assert!(missing.is_empty(), "live ids missing from the history reply: {missing:?}");
}

#[tokio::test]
async fn a_sidecar_that_exits_is_reported_and_started_again() {
    // The sidecar dies before answering the second request.
    let a = adapter(Replay::from(0).dying_after(1));
    let mut rx = a.subscribe().await.unwrap();
    a.status().await; // request 1 is answered
    let err = a.create(CreateSession { radar_session_id: radar(), cwd: PathBuf::from("/tmp/radar-live/repo"), model: None, setup: Default::default() }).await;
    assert!(err.is_err(), "the request in flight when the sidecar exits fails, it is not retried silently");
    match rx.recv().await {
        Some(Update::Disconnected { reason }) => assert!(reason.contains("exited"), "{reason}"),
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

#[tokio::test]
async fn too_many_restarts_stop_the_restarting() {
    use radar_sessions_claude::link::MAX_RESTARTS;
    // A recording with only `ready` and no requests answered: every process dies at once.
    let a = adapter(Replay::from(0).dying_after(0).repeating());
    let mut tries = 0;
    loop {
        let s = a.status().await;
        tries += 1;
        if s.error.as_deref().is_some_and(|e| e.contains("keeps exiting")) {
            break;
        }
        assert!(tries < 10, "never gave up: {s:?}");
    }
    assert!(tries > MAX_RESTARTS - 1);
}
