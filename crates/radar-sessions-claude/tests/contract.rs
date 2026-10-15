//! The Claude Code adapter passes radar-sessions' shared contract kit, over a scripted stand-in for
//! the sidecar (tests/fake_sidecar). Time is virtual (`start_paused`): `settle` sleeps long enough
//! for every task, including the reconnect back-off, to run, and no wall clock is read.

#[path = "fake_sidecar/mod.rs"]
mod fake_sidecar;

use std::sync::Arc;
use std::time::Duration;

use radar_sessions::contract::{Harness, Scenario, run_all};
use radar_sessions::{Adapter, BoxFuture};
use radar_sessions_claude::{ClaudeAdapter, ClaudeConfig};

use fake_sidecar::{FakeSpawner, Shared, provider};

struct ClaudeHarness {
    shared: Shared,
}

impl ClaudeHarness {
    fn new() -> Self {
        Self { shared: provider(Scenario::Text) }
    }

    fn adapter_over(&self) -> Box<dyn Adapter> {
        Box::new(ClaudeAdapter::new(Box::new(FakeSpawner(Arc::clone(&self.shared))), ClaudeConfig::default(), Arc::new(|| 1_000)))
    }
}

impl Harness for ClaudeHarness {
    fn adapter(&mut self, scenario: Scenario) -> BoxFuture<'_, Box<dyn Adapter>> {
        self.shared = provider(scenario);
        let a = self.adapter_over();
        Box::pin(async move { a })
    }

    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>> {
        let a = self.adapter_over();
        Box::pin(async move { a })
    }

    fn settle(&mut self) -> BoxFuture<'_, ()> {
        // Virtual time: the runtime runs every ready task, then jumps the clock.
        Box::pin(async { tokio::time::sleep(Duration::from_secs(40)).await })
    }

    fn disconnect(&mut self) -> BoxFuture<'_, ()> {
        let mut p = self.shared.lock().unwrap();
        p.connected = false;
        p.epoch.send_modify(|n| *n += 1);
        Box::pin(async {})
    }

    fn reconnect(&mut self) -> BoxFuture<'_, ()> {
        self.shared.lock().unwrap().connected = true;
        Box::pin(async {})
    }

    fn release(&mut self) -> BoxFuture<'_, ()> {
        // A hanging turn finishes. With the connection down no process is listening, so the
        // provider finishes it into its own history, as Claude Code would.
        let (connected, radar) = {
            let p = self.shared.lock().unwrap();
            (p.connected, p.sessions.iter().filter(|(_, h)| h.running).map(|(g, _)| g.clone()).collect::<Vec<_>>())
        };
        if connected {
            self.shared.lock().unwrap().release.send_modify(|n| *n += 1);
        } else {
            let mut p = self.shared.lock().unwrap();
            for g in radar {
                let h = p.sessions.get_mut(&g).unwrap();
                let pid = format!("provider-{g}");
                for (kind, id, payload) in [
                    ("assistant_message", "a-finished", serde_json::json!({"message_id":"m-finished","text":"Finished"})),
                    ("turn_end", "end-finished", serde_json::json!({"subtype":"success","is_error":false})),
                ] {
                    h.events.push(serde_json::json!({"type":"event","session":g,"provider_session_id":pid,"provider_event_id":id,"durability":"persisted","kind":kind,"payload":payload}));
                }
                h.running = false;
            }
        }
        Box::pin(async {})
    }
}

#[tokio::test(start_paused = true)]
async fn a_session_reports_the_slash_commands_claude_code_accepts() {
    use radar_sessions::{CreateSession, RadarSessionId, SlashCommand, Update};
    let h = ClaudeHarness::new();
    let adapter = h.adapter_over();
    let mut updates = adapter.subscribe().await.unwrap();
    // Nothing is promised before the session says: the menu starts empty.
    assert!(adapter.capabilities().slash_commands.is_empty());
    let radar = RadarSessionId::new("g1");
    adapter.create(CreateSession { radar_session_id: radar.clone(), cwd: std::env::temp_dir(), model: None, setup: Default::default() }).await.unwrap();
    tokio::time::sleep(Duration::from_secs(40)).await;
    let mut got = None;
    while let Ok(u) = updates.try_recv() {
        if let Update::Commands { session, commands } = u {
            assert_eq!(session, radar);
            got = Some(commands);
        }
    }
    assert_eq!(
        got.expect("the session reported its commands"),
        vec![SlashCommand::new("compact", "Free up context").with_hint("[instructions]"), SlashCommand::new("context", "Show context usage")]
    );
}

#[tokio::test(start_paused = true)]
async fn the_claude_adapter_passes_every_contract_check() {
    let report = run_all(&mut ClaudeHarness::new()).await;
    assert!(report.skipped.is_empty(), "nothing is skipped: {:?}", report.skipped);
    assert_eq!(report.ran.len(), 12, "{:?}", report.ran);
    assert!(report.ran.contains(&"question"), "{:?}", report.ran);
    assert!(report.ran.contains(&"approve_for_session"), "{:?}", report.ran);
}

#[tokio::test(start_paused = true)]
async fn approve_for_session_asks_the_sidecar_to_keep_claudes_rules_for_the_session() {
    let mut h = ClaudeHarness::new();
    radar_sessions::contract::approve_for_session(&mut h).await;
    let answers: Vec<_> = h.shared.lock().unwrap().requests.iter().filter(|r| r["op"] == "answer").cloned().collect();
    assert_eq!((answers[0]["behavior"].as_str(), answers[0]["scope"].as_str()), (Some("allow"), Some("session")), "{answers:?}");
}
