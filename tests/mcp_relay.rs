//! `radar mcp`, the real binary, between a CLI's stdio and a session's MCP listener
//! (radar-sessions `tools`): what an agent CLI starts as radar's MCP server, without a CLI.

#![cfg(feature = "sessions")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use radar_sessions::RadarSessionId;
use radar_sessions::tools::{McpListener, ToolCallSeen};
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-mcp-relay-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn radar() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_radar"))
}

#[test]
fn a_cli_reaches_the_session_through_radar_mcp_and_the_call_is_seen() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _in_rt = rt.enter();
    let dir = scratch("ok");
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let listener = McpListener::bind(dir.join("s.sock"), RadarSessionId::new("s"), tx).unwrap();
    let server = listener.tool_server(radar());

    let mut child = Command::new(&server.command)
        .args(&server.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |msg: Value| -> Value {
        writeln!(stdin, "{msg}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    let init = ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "radar");
    let list = ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    assert_eq!(list["result"]["tools"][0]["name"], "echo");
    let call = ask(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "echo", "arguments": { "text": "ping" } } }));
    assert_eq!(call["result"]["content"][0], json!({ "type": "text", "text": "ping" }));
    // The listener notes the call before it answers it.
    assert_eq!(
        seen.try_recv().unwrap(),
        ToolCallSeen { session: RadarSessionId::new("s"), tool: "echo".into(), arguments: json!({ "text": "ping" }) }
    );

    // The CLI closing stdin ends the relay, cleanly.
    drop(stdin);
    assert!(child.wait().unwrap().success());
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_ticket_is_closed_and_bad_arguments_say_how() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _in_rt = rt.enter();
    let dir = scratch("bad");
    let (tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let listener = McpListener::bind(dir.join("s.sock"), RadarSessionId::new("s"), tx).unwrap();

    let mut child = Command::new(radar())
        .args(["mcp", "--socket"])
        .arg(listener.path())
        .args(["--ticket", "not-it"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // The relay may already be gone (the listener closes at the ticket), so the write may fail.
    let _ = writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "echo", "arguments": { "text": "x" } } }));
    let mut out = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut out).unwrap();
    assert_eq!(out, "", "no answer to a wrong ticket");
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert!(seen.try_recv().is_err());

    let usage = Command::new(radar()).args(["mcp", "--socket", "/nowhere"]).output().unwrap();
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("usage: radar mcp"));
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);
}
