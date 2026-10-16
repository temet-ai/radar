//! Send to agent, as the dock sees it: contexts arrive from the host and are
//! sent at once, as the first message of a new session.

use radar_core::agent_context::{AgentContext, ContextField};
use radar_core::dock::{DockItem, DockView};
use radar_core::journal::SubjectKey;
use radar_core::{Outcome, Plugin};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::FakeProvider;

/// The plugin's tasks run on the test's thread, only when a step lets them: nothing races the
/// test, so no load on the machine makes it late.
fn plugin(provider: &FakeProvider) -> SessionsPlugin {
    let mut p = SessionsPlugin::on_this_thread();
    p.add_adapter(Box::new(provider.adapter()));
    // The first tick starts the plugin; a send waits for the subscription by itself.
    p.tick(0);
    p
}

/// Tick and run the plugin's tasks until `done`: a bound on steps, not on time.
fn until(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> DockView {
    for n in 0..256u64 {
        p.tick(n as i64);
        if let Some(dock) = p.dock()
            && done(&dock)
        {
            return dock;
        }
        p.run_tasks();
    }
    panic!("never reached: {what}; dock = {:?}", p.dock());
}

fn context() -> AgentContext {
    AgentContext::new("ticket", SubjectKey::new("sample", "https://one.example.test", "card", "ABC-1").unwrap(), "Fix the login")
        .field(ContextField::new("description", "Users are logged out after a minute"))
}

fn user_texts(d: &DockView) -> Vec<String> {
    d.transcript.iter().filter_map(|i| if let DockItem::User { text, .. } = i { Some(text.clone()) } else { None }).collect()
}

#[test]
fn sending_to_an_agent_starts_a_session_and_sends_the_brief_at_once() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    assert!(matches!(p.agent_brief(vec![context()]), Outcome::Handled(_)));
    let dock = until(&mut p, "the brief to arrive as the first message", |d| !user_texts(d).is_empty());
    assert_eq!(dock.tabs.len(), 1);
    assert_eq!(dock.composer.draft, "", "nothing is left waiting in the composer");
    let texts = user_texts(&dock);
    assert!(texts[0].contains("## Ticket: Fix the login") && texts[0].contains("Description: Users are logged out after a minute"), "{texts:?}");
    assert!(!texts[0].contains("untrusted"));
}

/// A CLI slow to listen: its subscription is held until the test lets it go. The plugin's tasks
/// run only when the test runs them, in the order they were spawned: the subscription first
/// (it waits), then the new session's. Created and sent at once, the session's answer to the
/// brief would be told to nobody; the brief waits until the adapter listens, then it is the
/// session's first message, once.
#[test]
fn the_brief_waits_for_the_adapter_to_listen_and_arrives_first_once() {
    let provider = FakeProvider::new();
    provider.hold_subscribe();
    let mut p = plugin(&provider);
    // The first tick asked for the subscription; the send comes before it is answered.
    assert!(matches!(p.agent_brief(vec![context()]), Outcome::Handled(_)));
    p.run_tasks();
    p.tick(1);
    p.run_tasks();
    assert!(!provider.listening());
    assert_eq!(provider.calls("create"), 0, "no session is made before the adapter listens");
    assert_eq!(provider.unheard(), 0);
    provider.let_subscribe();
    // Then the session is made, the brief sent and the fake's turn says "ok".
    let dock = until(&mut p, "the brief answered", |d| d.transcript.iter().any(|i| matches!(i, DockItem::Assistant { .. })));
    assert_eq!(provider.unheard(), 0, "nothing the session said was lost");
    assert_eq!(provider.calls("create"), 1);
    let texts = user_texts(&dock);
    assert_eq!(texts.len(), 1, "the brief is sent once: {texts:?}");
    assert!(texts[0].contains("## Ticket: Fix the login"));
    assert!(matches!(dock.transcript.first(), Some(DockItem::User { .. })), "the brief comes first: {:?}", dock.transcript);
}

#[test]
fn a_second_send_starts_a_second_session() {
    let provider = FakeProvider::new();
    let mut p = plugin(&provider);
    p.agent_brief(vec![context()]);
    until(&mut p, "the first brief", |d| !user_texts(d).is_empty());
    p.agent_brief(vec![context()]);
    let dock = until(&mut p, "the second brief", |d| d.tabs.len() == 2 && !user_texts(d).is_empty());
    assert_eq!(user_texts(&dock).len(), 1);
}
