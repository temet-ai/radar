//! What the adapter sends the sidecar for a session's setup and the provider's setup, and how it
//! reads the sidecar's catalog. The sidecar's own mapping onto the SDK is tested in
//! sidecars/claude-agent/test/setup.test.js.

#[path = "fake_sidecar/mod.rs"]
mod fake_sidecar;

use std::path::PathBuf;
use std::sync::Arc;

use radar_sessions::adapter::{CatalogEntry, ProviderSetup, SessionSetup};
use radar_sessions::contract::Scenario;
use radar_sessions::{Adapter, CreateSession, RadarSessionId, ResumeSession};
use radar_sessions_claude::{ClaudeAdapter, ClaudeConfig};
use serde_json::{Value, json};

use fake_sidecar::{FakeSpawner, Shared, provider};

fn adapter() -> (ClaudeAdapter, Shared) {
    let shared = provider(Scenario::Text);
    let a = ClaudeAdapter::new(Box::new(FakeSpawner(Arc::clone(&shared))), ClaudeConfig::default(), Arc::new(|| 1_000));
    (a, shared)
}

fn setup() -> SessionSetup {
    SessionSetup {
        extra_dirs: vec![PathBuf::from("/e")],
        agent: Some("scratch-reviewer".into()),
        skill_dirs: vec![PathBuf::from("/s")],
        skills: vec!["scratch-notes".into()],
        instruction_dirs: vec![PathBuf::from("/i")],
        plugin_dirs: vec![PathBuf::from("/p")],
        project_config: true,
        local_only: true,
        review: false,
        tool_server: Some(radar_sessions::ToolServer {
            name: "radar".into(),
            command: "/opt/radar".into(),
            args: vec!["mcp".into(), "--socket".into(), "/run/radar/mcp/g1.sock".into(), "--ticket".into(), "t0".into()],
            tools: vec!["echo".into()],
        }),
    }
}

fn tool_server_json() -> Value {
    json!({ "name": "radar", "command": "/opt/radar", "args": ["mcp", "--socket", "/run/radar/mcp/g1.sock", "--ticket", "t0"], "tools": ["echo"] })
}

fn last(shared: &Shared, op: &str) -> Value {
    shared.lock().unwrap().requests.iter().rev().find(|r| r["op"] == op).cloned().unwrap_or_else(|| panic!("no {op} request"))
}

#[tokio::test]
async fn capabilities_say_agents_and_skills_are_real_and_what_is_not() {
    let (a, _) = adapter();
    let c = a.capabilities();
    assert!(c.agents && c.skills);
    assert!(c.unsupported.iter().any(|u| u.control == "local_only"), "local_only is declared unsupported");
    // A folder's CLAUDE.md loads natively and its AGENTS.md is given by the sidecar.
    assert!(!c.unsupported.iter().any(|u| u.control == "instruction_dirs"));
}

#[tokio::test]
async fn create_and_resume_send_the_setup_whole() {
    let (a, shared) = adapter();
    let radar = RadarSessionId::new("g1");
    a.create(CreateSession { radar_session_id: radar.clone(), cwd: PathBuf::from("/work"), model: None, setup: setup() }).await.unwrap();
    let r = last(&shared, "create");
    assert_eq!(r["cwd"], "/work");
    assert_eq!(r["setup"]["extra_dirs"], json!(["/e"]));
    assert_eq!(r["setup"]["agent"], "scratch-reviewer");
    assert_eq!(r["setup"]["skill_dirs"], json!(["/s"]));
    assert_eq!(r["setup"]["skills"], json!(["scratch-notes"]));
    assert_eq!(r["setup"]["instruction_dirs"], json!(["/i"]));
    assert_eq!(r["setup"]["plugin_dirs"], json!(["/p"]));
    assert_eq!(r["setup"]["project_config"], true);
    assert_eq!(r["setup"]["local_only"], true);
    // No ask rule from radar: the sidecar leaves the operator's own Claude Code rules in charge.
    assert!(r.get("ask").is_none(), "{r}");
    assert_eq!(r["setup"]["review"], false);
    // radar's own MCP server, which the sidecar maps to mcpServers and allowedTools.
    assert_eq!(r["setup"]["tool_server"], tool_server_json());
    assert!(r["session_id"].is_string() && r["resume"].is_null());

    // A new process (the sidecar forgot the session) is resumed with the same setup.
    let b = ClaudeAdapter::new(Box::new(FakeSpawner(Arc::clone(&shared))), ClaudeConfig::default(), Arc::new(|| 1_000));
    b.resume(ResumeSession {
        radar_session_id: radar,
        provider_session_id: radar_sessions::ProviderSessionId::new("provider-g1"),
        cwd: PathBuf::from("/work"),
        cursor: None,
        setup: setup(),
    })
    .await
    .unwrap();
    let r = last(&shared, "create");
    assert_eq!(r["resume"], "provider-g1");
    assert_eq!(r["setup"]["agent"], "scratch-reviewer");
    assert_eq!(r["setup"]["project_config"], true);
    assert_eq!(r["setup"]["tool_server"], tool_server_json());
}

#[tokio::test]
async fn an_empty_setup_asks_for_no_project_config() {
    let (a, shared) = adapter();
    a.create(CreateSession { radar_session_id: RadarSessionId::new("g2"), cwd: PathBuf::from("/w"), model: None, setup: SessionSetup::default() }).await.unwrap();
    let r = last(&shared, "create");
    assert_eq!(r["setup"]["project_config"], false);
    assert!(r["setup"]["agent"].is_null());
    assert!(r["setup"].get("tool_server").is_none(), "no server: nothing sent");
    assert!(r.get("config_dir").is_none() && r.get("claude_path").is_none());
}

#[tokio::test]
async fn configure_applies_to_the_next_request_it_does_not_rewrite_the_last() {
    let (a, shared) = adapter();
    a.create(CreateSession { radar_session_id: RadarSessionId::new("g3"), cwd: PathBuf::from("/w"), model: None, setup: SessionSetup::default() }).await.unwrap();
    assert!(last(&shared, "create").get("config_dir").is_none());
    a.configure(ProviderSetup { executable: Some(PathBuf::from("/opt/claude")), config_dir: Some(PathBuf::from("/cfg")) });
    a.create(CreateSession { radar_session_id: RadarSessionId::new("g4"), cwd: PathBuf::from("/w"), model: None, setup: SessionSetup::default() }).await.unwrap();
    let r = last(&shared, "create");
    assert_eq!(r["claude_path"], "/opt/claude");
    assert_eq!(r["config_dir"], "/cfg");
    // Everything that asks `claude` something carries them.
    a.status().await;
    assert_eq!(last(&shared, "status")["config_dir"], "/cfg");
    a.list_models().await.unwrap();
    assert_eq!(last(&shared, "models")["claude_path"], "/opt/claude");
    a.list_sessions().await.unwrap();
    assert_eq!(last(&shared, "list")["config_dir"], "/cfg");
    // Cleared again.
    a.configure(ProviderSetup::default());
    a.list_sessions().await.unwrap();
    assert!(last(&shared, "list").get("config_dir").is_none());
}

#[tokio::test]
async fn catalog_sends_cwd_and_setup_and_maps_the_entries() {
    let (a, shared) = adapter();
    let c = a.catalog(PathBuf::from("/work"), setup()).await.unwrap();
    let r = last(&shared, "catalog");
    assert_eq!(r["cwd"], "/work");
    assert_eq!(r["setup"]["skill_dirs"], json!(["/s"]));
    assert!(r["session"].is_null(), "a catalog is no session");
    assert_eq!(
        c.agents,
        vec![
            CatalogEntry { name: "scratch-reviewer".into(), description: Some("Reviews".into()), source: Some("project".into()) },
            CatalogEntry { name: "Explore".into(), description: None, source: None },
        ]
    );
    assert_eq!(c.skills, vec![CatalogEntry { name: "scratch-notes".into(), description: Some("Notes".into()), source: Some("project".into()) }], "an entry with no name is dropped");
}

#[tokio::test]
async fn warm_starts_the_sidecar_without_a_session_and_an_unused_session_can_be_discarded() {
    let (a, shared) = adapter();
    assert!(a.idle_session_is_free(), "Claude Code writes nothing for a session until its first message");
    a.warm().await.unwrap();
    {
        let p = shared.lock().unwrap();
        assert_eq!(p.requests.iter().map(|r| r["op"].as_str().unwrap_or_default()).collect::<Vec<_>>(), ["hello"], "the sidecar started, no session");
        assert!(p.sessions.is_empty());
    }
    let radar = RadarSessionId::new("g-spare");
    a.create(CreateSession { radar_session_id: radar.clone(), cwd: PathBuf::from("/work"), model: None, setup: SessionSetup::default() }).await.unwrap();
    a.discard(&radar).await.unwrap();
    assert_eq!(last(&shared, "close")["session"], "g-spare", "the sidecar closes its query, which ends the claude process");
    // Forgotten here: nothing can be sent to it any more.
    assert!(a.send(&radar, radar_sessions::SendInput { text: "hi".into() }).await.is_err());
}
