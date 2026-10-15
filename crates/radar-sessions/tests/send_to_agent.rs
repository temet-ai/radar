//! Send to agent, as the dock sees it: contexts arrive from the host and are
//! sent at once, as the first message of a new session.

use radar_core::agent_context::{AgentContext, ContextField};
use radar_core::dock::{DockItem, DockView};
use radar_core::journal::SubjectKey;
use radar_core::{Outcome, Plugin};
use radar_sessions::SessionsPlugin;
use radar_sessions::fake::FakeProvider;

fn plugin(provider: &FakeProvider) -> SessionsPlugin {
    let mut p = SessionsPlugin::new();
    p.add_adapter(Box::new(provider.adapter()));
    // The host ticks long before anyone sends: the plugin has started and subscribed.
    for n in 0..50 {
        p.tick(n);
    }
    p
}

fn until(p: &mut SessionsPlugin, what: &str, done: impl Fn(&DockView) -> bool) -> DockView {
    for n in 0..2_000_000u64 {
        p.tick(n as i64);
        if let Some(dock) = p.dock()
            && done(&dock)
        {
            return dock;
        }
        std::thread::yield_now();
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
