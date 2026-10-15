//! Send to agent in the host: the part that owns the subject gives its context,
//! the part that owns the dock gets the contexts, and the dock opens.
use super::*;
use crate::agent_context::{AgentContext, AgentContextRequest, ContextField};
use crate::dock::DockView;
use crate::journal::SubjectKey;
use std::sync::{Arc, Mutex};

fn key() -> SubjectKey {
    SubjectKey::new("owner", "https://one.example.test", "card", "A-1").unwrap()
}

struct Owner {
    answers: bool,
}
impl Plugin for Owner {
    fn id(&self) -> &'static str {
        "owner"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn agent_context(&mut self, request: &AgentContextRequest) -> Option<AgentContext> {
        (self.answers && request.subject == key())
            .then(|| AgentContext::new("card", key(), "A card").field(ContextField::new("body", "text")))
    }
}

struct Docked {
    got: Arc<Mutex<Vec<AgentContext>>>,
}
impl Plugin for Docked {
    fn id(&self) -> &'static str {
        "docked"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn dock(&self) -> Option<DockView> {
        Some(DockView::default())
    }
    fn agent_brief(&mut self, contexts: Vec<AgentContext>) -> Outcome {
        self.got.lock().unwrap().extend(contexts);
        Outcome::done()
    }
}

fn host(answers: bool) -> (Host, Arc<Mutex<Vec<AgentContext>>>) {
    let got = Arc::new(Mutex::new(vec![]));
    let parts: Vec<Box<dyn Plugin>> = vec![Box::new(Owner { answers }), Box::new(Docked { got: got.clone() })];
    (Host::new(parts, 0).unwrap(), got)
}

#[test]
fn send_to_agent_gives_the_owners_context_to_the_dock_and_opens_it() {
    let (mut host, got) = host(true);
    host.route(0, vec![Effect::SendToAgent(AgentContextRequest { subject: key() })], 0);
    let got = got.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].title, "A card");
    let dock = host.dock_view().unwrap();
    assert!(dock.visible && dock.focused);
}

#[test]
fn send_to_agent_with_no_context_says_so_and_opens_nothing() {
    let (mut host, got) = host(false);
    host.route(0, vec![Effect::SendToAgent(AgentContextRequest { subject: key() })], 0);
    assert!(got.lock().unwrap().is_empty());
    assert!(!host.dock_view().unwrap().visible);
}
