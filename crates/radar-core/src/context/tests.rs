use super::*;
use crate::agent_context::ContextField;
use crate::{
    Owned,
    journal::{SubjectRole, SubjectSnapshot},
};
use std::sync::{Arc, Mutex};

struct Source {
    body: Arc<Mutex<String>>,
}
fn key(id: &str) -> SubjectKey {
    SubjectKey::new("sample", "https://watch.example.test", "item", id).unwrap()
}
impl Plugin for Source {
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn id(&self) -> &'static str {
        "context-fixture"
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("item", "Items")]
    }
    fn agent_subjects(&self) -> Vec<Subject> {
        ["WATCH-12", "WATCH-13"]
            .into_iter()
            .map(|id| Subject::new("item", id, id))
            .collect()
    }
    fn journal_subject(&self, s: &Subject) -> Option<SubjectSnapshot> {
        self.agent_subjects()
            .iter()
            .any(|t| t.id == s.id)
            .then(|| SubjectSnapshot::new(key(&s.id), SubjectRole::Primary))
    }
    fn subject_from_journal(&self, k: &SubjectKey) -> Option<Subject> {
        self.agent_subjects().into_iter().find(|s| key(&s.id) == *k)
    }
    fn agent_document(&mut self, r: &AgentContextRequest) -> Option<Document> {
        let s = self.subject_from_journal(&r.subject)?;
        Some(
            Document::from_context(
                AgentContext::new("Item", r.subject.clone(), &s.label)
                    .field(ContextField::new("description", "short preview")),
            )
            .section("description", self.body.lock().unwrap().clone()),
        )
    }
    fn agent_related(&self, s: &Subject) -> Vec<Related> {
        self.agent_subjects()
            .into_iter()
            .filter(|r| r.id != s.id)
            .map(|s| Related::new(s, "linked", "provider", "fixture issue link"))
            .collect()
    }
}
fn setup() -> (Service, Vec<Box<dyn Plugin>>, Arc<Mutex<String>>) {
    let body = Arc::new(Mutex::new(format!(
        "{} ghp_123456789012345678901234567890 secret tail",
        "🦉 context ".repeat(1800)
    )));
    (
        Service::default(),
        vec![Box::new(Source { body: body.clone() })],
        body,
    )
}
#[test]
fn connected_context_is_scoped_deduplicated_and_readable_after_source_changes() {
    let (mut service, mut plugins, body) = setup();
    let root = reference(&key("WATCH-12"));
    let context = service
        .execute(&mut plugins, "context", json!({"roots":[root,root]}), 100)
        .unwrap();
    assert_eq!(context["known_items"], 2);
    assert!(
        context["relations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["basis"] == "provider")
    );
    let snapshot = context["snapshot_id"].clone();
    *body.lock().unwrap() = "changed requirements".into();
    let mut page = service.execute(&mut plugins, "read", json!({"refs":[root],"snapshot_id":snapshot,"sections":["description"],"max_bytes":1024}), 101).unwrap();
    assert_eq!(
        page["sources"][0]["revision"],
        context["items"][0]["revision"]
    );
    let mut all = String::new();
    loop {
        all.push_str(page["text"].as_str().unwrap());
        assert!(page["bytes"].as_u64().unwrap() <= 1024);
        let Some(cursor) = page["next_cursor"].as_str() else {
            break;
        };
        page = service
            .execute(
                &mut plugins,
                "read",
                json!({"cursor":cursor,"max_bytes":1024}),
                102,
            )
            .unwrap();
    }
    assert!(
        all.contains("🦉 context") && all.contains("secret tail") && all.contains("[redacted]")
    );
    assert!(!all.contains("ghp_") && !all.contains("changed requirements"));
    let changes = service
        .execute(
            &mut plugins,
            "changes",
            json!({"snapshot_id":snapshot}),
            103,
        )
        .unwrap();
    assert_eq!(changes["changed_refs"].as_array().unwrap().len(), 2);
    plugins.clear();
    assert!(
        service
            .execute(
                &mut plugins,
                "read",
                json!({"refs":[root],"snapshot_id":snapshot}),
                104
            )
            .is_err()
    );
}
#[test]
fn exact_search_refs_and_cursor_expiry_are_explicit() {
    let (mut service, mut plugins, _) = setup();
    let root = reference(&key("WATCH-12"));
    let found = service
        .execute(&mut plugins, "search", json!({"query":root}), 100)
        .unwrap();
    assert_eq!(found["known_items"], 1);
    let context = service
        .execute(
            &mut plugins,
            "context",
            json!({"selection_ref":found["selection_ref"]}),
            101,
        )
        .unwrap();
    assert_eq!(context["known_items"], 2);
    assert!(
        service
            .execute(
                &mut plugins,
                "changes",
                json!({"snapshot_id":context["snapshot_id"]}),
                4000
            )
            .unwrap_err()
            .contains("expired")
    );
}

#[test]
fn changes_pages_use_the_changes_tool_and_unused_filters_are_rejected() {
    let (mut service, mut plugins, _) = setup();
    let packet = service
        .execute(
            &mut plugins,
            "context",
            json!({"roots":[reference(&key("WATCH-12"))]}),
            100,
        )
        .unwrap();
    let changed = service
        .execute(
            &mut plugins,
            "changes",
            json!({"snapshot_id":packet["snapshot_id"],"max_bytes":1024}),
            101,
        )
        .unwrap();
    if let Some(cursor) = changed["next_cursor"].as_str() {
        assert!(cursor.starts_with("changes:"));
        service
            .execute(&mut plugins, "changes", json!({"cursor":cursor}), 102)
            .unwrap();
    }
    assert!(
        service
            .execute(
                &mut plugins,
                "context",
                json!({"roots":[reference(&key("WATCH-12"))],"assignee":"self"}),
                103
            )
            .unwrap_err()
            .contains("does not accept assignee")
    );
}
