//! A few process-boundary checks; detailed behavior lives in radar-core tests.

use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "radar-work-process-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    fn command(&self, args: &[&str], mode: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_radar"));
        command
            .args(args)
            .env_clear()
            .env("RADAR_SANDBOX", "1")
            .env("HOME", self.0.join("home"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("RADAR_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn run(&self, name: &str, input: &Value, mode: &str) -> Output {
        let mut child = self
            .command(&["work", name, "--stdin"], mode)
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
    fn json(&self, name: &str, input: &Value, mode: &str) -> Value {
        let output = self.run(name, input, mode);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn creation() -> Value {
    json!({"version":1,"changes":[{"change_id":"11111111-1111-4111-8111-111111111111","basis":"user_asserted","evidence":{"type":"assertion"},"mutation":{"type":"create_item","item_id":"22222222-2222-4222-8222-222222222222","kind":"card"}}]})
}
fn items() -> Value {
    json!({"version":1,"filter":{"all_tags":[]},"limit":10,"cursor":null})
}

#[test]
fn help_id_and_invalid_input_have_no_state_side_effects_or_secret_echoes() {
    let sandbox = Sandbox::new();
    let help = sandbox
        .command(&["work", "--help"], "read-only")
        .output()
        .unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("record their own invocation"));
    let id = sandbox
        .command(&["work", "id"], "read-only")
        .output()
        .unwrap();
    assert!(id.status.success());
    let id = String::from_utf8(id.stdout).unwrap();
    assert_eq!(id.trim().len(), 36);
    assert!(!sandbox.0.join("state").exists());
    let invalid = sandbox
        .command(&["work", "--secret-argv-value"], "read-only")
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("secret-argv-value"));
    let invalid = sandbox.run(
        "items",
        &json!({"secret-field":"secret-content"}),
        "read-only",
    );
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("secret-content"));
    assert!(!sandbox.0.join("state").exists());
}

#[cfg(unix)]
#[test]
fn process_retry_is_idempotent_and_audited_handoff_keeps_its_cut() {
    let sandbox = Sandbox::new();
    let first = sandbox.json("apply", &creation(), "write");
    let again = sandbox.json("apply", &creation(), "write");
    assert_eq!(
        first["changes"][0]["sequence"],
        again["changes"][0]["sequence"]
    );
    assert_eq!(again["changes"][0]["replayed"], true);
    assert_ne!(first["attempt"]["action_id"], again["attempt"]["action_id"]);
    let first_items = sandbox.json("items", &items(), "read-only");
    let second_items = sandbox.json("items", &items(), "read-only");
    assert_eq!(second_items["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        first_items["snapshot"]["map_sequence"],
        second_items["snapshot"]["map_sequence"]
    );
    assert!(
        second_items["snapshot"]["event_sequence"].as_i64().unwrap()
            > first_items["snapshot"]["event_sequence"].as_i64().unwrap()
    );
    let mut input = json!({"version":1,"selection":{"items":["22222222-2222-4222-8222-222222222222"],"actions":[first["attempt"]["action_id"]]},"records":1,"bytes":4096,"cursor":null});
    let output = sandbox.run("handoff", &input, "read-only");
    assert!(output.status.success());
    assert!(output.stdout.len() <= 4096);
    assert!(!output.stdout.ends_with(b"\n"));
    let page: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(page["authority"], "observations_only");
    assert_eq!(page["execution_authorization"], "unspecified");
    assert_eq!(page["truncated"], true);
    input["cursor"] = page["next"].clone();
    let next = sandbox.json("handoff", &input, "read-only");
    assert_eq!(page["snapshot"], next["snapshot"]);
    assert!(sandbox.0.join("state/radar/journal/actions.sqlite3").is_file());
    assert!(sandbox.0.join("state/radar/radar.log").is_file());
}

#[cfg(unix)]
#[test]
fn mode_denial_is_audited_and_non_utf8_environment_does_not_panic() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let sandbox = Sandbox::new();
    let denied = sandbox.run("apply", &creation(), "read-only");
    assert!(!denied.status.success());
    let page = sandbox.json("items", &items(), "read-only");
    assert!(page["items"].as_array().unwrap().is_empty());
    assert!(page["snapshot"]["event_sequence"].as_i64().unwrap() > 0);
    let mut command = sandbox.command(&["work", "items", "--stdin"], "read-only");
    command.env(
        "UNRELATED",
        OsString::from_vec(b"private-nonutf8-\xff".to_vec()),
    );
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(items().to_string().as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("private-nonutf8"));
}

#[cfg(unix)]
#[test]
fn process_discovers_real_unbound_evidence_before_observed_binding_and_handoff() {
    use radar_core::journal::{
        ActionDescriptor, Journal, Metadata, Origin, Outcome, SessionOrigin, SubjectKey,
        SubjectRole, SubjectSnapshot,
    };
    let sandbox = Sandbox::new();
    let path = sandbox.0.join("state/radar/journal/actions.sqlite3");
    let subject = SubjectKey::new("sample", "https://one.example.test", "card", "42").unwrap();
    let mut journal = Journal::open(&path).unwrap();
    let session = journal.start_session_with(SessionOrigin::Tui).unwrap();
    let action = journal
        .begin(
            &session,
            ActionDescriptor::new("card.inspect", Origin::Plugin("sample".into()))
                .with_subject(SubjectSnapshot::new(subject.clone(), SubjectRole::Primary)),
            None,
        )
        .unwrap();
    let evidence = journal
        .record(
            &action,
            Outcome::Partial,
            Metadata {
                truncated: true,
                ..Metadata::default()
            },
        )
        .unwrap();
    journal.finish_session(&session).unwrap();
    drop(journal);
    let query = json!({"version":1,"filter":{"session_id":null,"action_id":null,"root_action_id":null,"kind":"card.inspect","subject":subject,"outcome":"partial","since_ms":null,"until_ms":null},"records":10,"bytes":8192,"cursor":null});
    let output = sandbox.run("events", &query, "read-only");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 8192 && !output.stdout.ends_with(b"\n"));
    let page: Value = serde_json::from_slice(&output.stdout).unwrap();
    let entry = &page["events"][0];
    assert_eq!(entry["event"]["event_id"], evidence.as_str());
    assert_eq!(entry["source"]["session_origin"], "tui");
    assert!(
        entry["requested_sequence"].as_i64().unwrap()
            < entry["event"]["sequence"].as_i64().unwrap()
    );
    assert!(
        sandbox.json("items", &items(), "read-only")["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut apply = creation();
    apply["changes"].as_array_mut().unwrap().push(json!({"change_id":"33333333-3333-4333-8333-333333333333","basis":"observed","evidence":{"type":"event","event_id":entry["event"]["event_id"]},"mutation":{"type":"bind_subject","item_id":"22222222-2222-4222-8222-222222222222","subject":subject,"from_sequence":entry["requested_sequence"],"through_sequence":entry["requested_sequence"]}}));
    sandbox.json("apply", &apply, "write");
    let handoff = sandbox.json("handoff", &json!({"version":1,"selection":{"items":["22222222-2222-4222-8222-222222222222"],"actions":[]},"records":100,"bytes":262144,"cursor":null}), "read-only");
    let original = handoff["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["type"] == "action" && record["event"]["event_id"] == evidence.as_str()
        })
        .unwrap();
    assert_eq!(original["event"], entry["event"]);
    assert_eq!(original["association"], "explicit_binding");
    assert_eq!(handoff["authority"], "observations_only");
    assert_eq!(handoff["execution_authorization"], "unspecified");
}
