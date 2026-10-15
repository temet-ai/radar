//! A live check: does `codex app-server` take radar's
//! MCP server from a thread's own `config`, and does the thread call its `echo` tool?
//!
//!   cargo run -p radar-sessions --example mcp_listen -- <sock> <radar> <server.json> &
//!   cargo run -p radar-sessions-codex --example mcp_codex -- <scratch-repo> <out.jsonl> <server.json> <variant>
//!
//! `variant`:
//! - `config`: `thread/start` with `config` from [`setup::thread_config`] (radar's server, its
//!   tools named, nothing about approval), under radar's reviewer policy (`untrusted`,
//!   read-only);
//! - `approve`: the same plus `default_tools_approval_mode = "approve"` for radar's server;
//! - `dynamic`: no MCP server; `dynamicTools` (experimental API) with an `echo` function, each
//!   `item/tool/call` answered here.
//!
//! Then, in the same app-server, a second thread started without the server is asked for its
//! MCP servers (radar's must be absent). After the turn the app-server is stopped, a new one
//! resumes the thread with the same `config` (`thread/resume`), and a second turn calls the
//! tool again.
//!
//! Speaks the app-server's JSON-RPC itself (not through the adapter, which refuses MCP
//! elicitations and dynamic tool calls today), records every line both ways as
//! `{"dir","msg"}` like tests/fixtures/phase1.jsonl, and deletes the thread at the end. An
//! MCP elicitation (Codex's approval of an MCP call) is accepted once; any command or file
//! approval is declined. Scan the output with
//! `scripts/scan-secrets` before committing it.

use std::path::PathBuf;
use std::time::Duration;

use radar_sessions::ToolServer;
use radar_sessions::adapter::SessionSetup;
use radar_sessions_codex::SessionPolicy;
use radar_sessions_codex::setup::thread_config;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const LONG: Duration = Duration::from_secs(180);

struct Rpc {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next: u64,
}

struct Run {
    log: Vec<Value>,
    rpc: Rpc,
}

impl Run {
    async fn start(log: Vec<Value>, dynamic: bool) -> anyhow::Result<Self> {
        let mut child = Command::new("codex")
            .args(["app-server", "--disable", "hooks"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let rpc = Rpc {
            stdin: child.stdin.take().unwrap(),
            lines: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            next: 0,
        };
        let mut run = Self { log, rpc };
        let mut hello = json!({ "clientInfo": { "name": "radar", "title": "radar", "version": "0.0.0" } });
        if dynamic {
            hello["capabilities"] = json!({ "experimentalApi": true });
        }
        run.call("initialize", hello).await?;
        run.send(json!({ "method": "initialized" })).await?;
        Ok(run)
    }

    async fn stop(mut self) -> Vec<Value> {
        let _ = self.rpc.stdin.shutdown().await;
        let _ = self.rpc.child.kill().await;
        self.log
    }

    async fn send(&mut self, msg: Value) -> anyhow::Result<()> {
        self.log.push(json!({ "dir": "out", "msg": msg }));
        self.rpc.stdin.write_all(format!("{msg}\n").as_bytes()).await?;
        Ok(())
    }

    async fn recv(&mut self, wait: Duration) -> anyhow::Result<Option<Value>> {
        let Ok(line) = tokio::time::timeout(wait, self.rpc.lines.next_line()).await else {
            return Ok(None);
        };
        let Some(line) = line? else { anyhow::bail!("codex app-server closed its output") };
        let msg: Value = serde_json::from_str(&line)?;
        self.log.push(json!({ "dir": "in", "msg": msg }));
        Ok(Some(msg))
    }

    /// A request and its answer; what comes before the answer is logged and passed over.
    async fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.rpc.next;
        self.rpc.next += 1;
        self.send(json!({ "method": method, "id": id, "params": params })).await?;
        loop {
            let m = self.recv(LONG).await?.ok_or_else(|| anyhow::anyhow!("no answer to {method}"))?;
            if m["id"] == id && m.get("method").is_none() {
                if let Some(e) = m.get("error") {
                    anyhow::bail!("{method} refused: {e}");
                }
                return Ok(m["result"].clone());
            }
            if m["method"] == "mcpServer/startupStatus/updated" {
                eprintln!("startup: {}", m["params"]);
            }
        }
    }

    /// Waits (at most 20 s: Codex starts MCP servers without blocking the thread) for `name`
    /// to be ready or to fail.
    async fn wait_started(&mut self, name: &str) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while let Some(m) = self.recv(deadline.saturating_duration_since(tokio::time::Instant::now())).await? {
            if m["method"] == "mcpServer/startupStatus/updated" {
                eprintln!("startup: {}", m["params"]);
                if m.pointer("/params/name").and_then(Value::as_str) == Some(name)
                    && matches!(m.pointer("/params/status").and_then(Value::as_str), Some("ready" | "failed" | "cancelled"))
                {
                    return Ok(());
                }
            }
        }
        eprintln!("no start-up status for `{name}` within 20 s");
        Ok(())
    }

    /// `name`'s entry in the thread's MCP servers, if it has one.
    async fn server_of(&mut self, thread: &str, name: &str) -> anyhow::Result<Option<Value>> {
        let r = self.call("mcpServerStatus/list", json!({ "threadId": thread, "detail": "toolsAndAuthOnly" })).await?;
        Ok(r["data"].as_array().into_iter().flatten().find(|s| s["name"] == name).cloned())
    }

    /// One turn, answering the server's requests, until `turn/completed`.
    async fn turn(&mut self, thread: &str, prompt: &str) -> anyhow::Result<()> {
        self.call("turn/start", json!({ "threadId": thread, "input": [{ "type": "text", "text": prompt, "text_elements": [] }] })).await?;
        loop {
            let Some(m) = self.recv(LONG).await? else {
                eprintln!("timeout");
                return Ok(());
            };
            let method = m["method"].as_str().unwrap_or_default().to_string();
            if m.get("id").is_some() && !method.is_empty() {
                let line = self.answer(&m).await?;
                eprintln!("{line}");
            } else if method == "item/started" || method == "item/completed" {
                let item = &m["params"]["item"];
                if !matches!(item["type"].as_str(), Some("agentMessage" | "reasoning" | "userMessage")) {
                    eprintln!("{method}: {}", item.to_string().chars().take(400).collect::<String>());
                }
            } else if method == "turn/completed" {
                eprintln!("turn/completed: {}", m["params"]["turn"]["status"]);
                return Ok(());
            }
        }
    }

    /// Answers a server request; returns a line about it for the operator.
    async fn answer(&mut self, msg: &Value) -> anyhow::Result<String> {
        let method = msg["method"].as_str().unwrap_or_default();
        let id = msg["id"].clone();
        let result = match method {
            "mcpServer/elicitation/request" => json!({ "action": "accept", "content": {} }),
            "item/tool/call" => {
                let text = msg.pointer("/params/arguments/text").and_then(Value::as_str).unwrap_or_default();
                json!({ "contentItems": [{ "type": "inputText", "text": text }], "success": true })
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" | "item/permissions/requestApproval" => {
                json!({ "decision": "decline" })
            }
            _ => {
                self.send(json!({ "id": id, "error": { "code": -32601, "message": "not answered in this check" } })).await?;
                return Ok(format!("refused server request {method}"));
            }
        };
        self.send(json!({ "id": id, "result": result })).await?;
        Ok(format!("answered {method}: {}", msg["params"].to_string().chars().take(300).collect::<String>()))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, repo, out, server, variant] = &args[..] else {
        anyhow::bail!("usage: mcp_codex <scratch-repo> <out.jsonl> <server.json> config|approve|dynamic");
    };
    let repo = PathBuf::from(repo);
    let server: ToolServer = serde_json::from_str(&std::fs::read_to_string(server)?)?;
    let policy = SessionPolicy::read_only();
    let dynamic = variant == "dynamic";

    // What thread/start and thread/resume carry besides the thread itself.
    let mut params = json!({
        "cwd": repo,
        "approvalPolicy": policy.approval_policy,
        "sandbox": policy.sandbox,
        "approvalsReviewer": "user",
    });
    if dynamic {
        params["dynamicTools"] = json!([{
            "type": "function",
            "name": "echo",
            "description": "radar's echo: returns the text it is given.",
            "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
        }]);
        if let Some(cfg) = thread_config(Some(&policy), &SessionSetup::default()) {
            params["config"] = cfg;
        }
    } else {
        let setup = SessionSetup { tool_server: Some(server.clone()), ..Default::default() };
        let mut cfg = thread_config(Some(&policy), &setup).expect("a tool server is config");
        if variant == "approve" {
            cfg["mcp_servers"][server.name.as_str()]["default_tools_approval_mode"] = json!("approve");
        }
        params["config"] = cfg;
    }

    let mut run = Run::start(Vec::new(), dynamic).await?;
    let started = run.call("thread/start", params.clone()).await?;
    eprintln!("thread/start echoed approvalPolicy={} sandbox={}", started["approvalPolicy"], started["sandbox"]);
    let thread = started.pointer("/thread/id").and_then(Value::as_str).unwrap_or_default().to_string();
    if !dynamic {
        run.wait_started(&server.name).await?;
        let radar = run.server_of(&thread, &server.name).await?;
        eprintln!("the thread's `{}`: {}", server.name, radar.as_ref().map_or("absent".into(), |s| format!("{} tools {:?}", s["runtimeStatus"], s["tools"].as_object().map(|t| t.keys().collect::<Vec<_>>()))));

        // Per session, not per process: a second thread in the same app-server, started
        // without the server, must not have it (ephemeral: it leaves nothing behind).
        let other = run
            .call("thread/start", json!({ "cwd": repo, "approvalPolicy": policy.approval_policy, "sandbox": policy.sandbox, "approvalsReviewer": "user", "ephemeral": true }))
            .await?;
        let other = other.pointer("/thread/id").and_then(Value::as_str).unwrap_or_default().to_string();
        eprintln!("a second thread without the server has `{}`: {}", server.name, run.server_of(&other, &server.name).await?.is_some());
    }
    run.turn(&thread, "Call the echo tool from the radar server with text 'ping' and tell me the result.").await?;

    // A new app-server resumes the thread with the same config: the server must come back.
    let log = run.stop().await;
    let mut run = Run::start(log, dynamic).await?;
    let mut resume = params.clone();
    resume["threadId"] = json!(thread);
    run.call("thread/resume", resume).await?;
    if !dynamic {
        run.wait_started(&server.name).await?;
        eprintln!("after thread/resume the thread has `{}`: {}", server.name, run.server_of(&thread, &server.name).await?.is_some());
    }
    run.turn(&thread, "Call the echo tool from the radar server again, with text 'pong', and tell me the result.").await?;
    run.call("thread/read", json!({ "threadId": thread, "includeTurns": true })).await?;
    run.call("thread/delete", json!({ "threadId": thread })).await?;
    let log = run.stop().await;

    let home = std::env::var("HOME").unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim().to_string();
    let ticket = server.args.last().cloned().unwrap_or_default();
    let socket = server.args.get(2).cloned().unwrap_or_default();
    let mut text: String = log.iter().map(|l| format!("{l}\n")).collect();
    for (from, to) in [
        (ticket.as_str(), "0123456789abcdef0123456789abcdef"),
        (socket.as_str(), "/run/user/1000/radar/mcp/spike.sock"),
        (server.command.as_str(), "/usr/local/bin/radar"),
        (&repo.display().to_string(), "/work/repo"),
        (thread.as_str(), "00000000-0000-4000-8000-000000000002"),
        (home.as_str(), "/home/user"),
    ] {
        if !from.is_empty() {
            text = text.replace(from, to);
        }
    }
    if !host.is_empty() {
        text = text.replace(&format!("\"{host}\""), "\"host\"");
    }
    std::fs::write(out, text)?;
    eprintln!("recorded {} lines to {out}", log.len());
    Ok(())
}
