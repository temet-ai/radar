//! The real MCP endpoint: bounded asynchronous calls, served by the host's
//! context service. The same endpoint is available to dock and external clients.
use std::collections::BTreeMap;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

use super::{McpListener, new_ticket, read_line};
use crate::adapter::ToolServer;
use radar_core::context::{Call, TOOLS};

type Answer = Result<Value, String>;
struct Pending {
    call: Call,
    answer: oneshot::Sender<Answer>,
}
type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Serialize, Deserialize)]
struct Endpoint {
    socket: PathBuf,
    ticket: String,
    workspace: PathBuf,
}

pub struct ContextServer {
    listener: McpListener,
    receiver: mpsc::Receiver<Pending>,
    pending: BTreeMap<u64, oneshot::Sender<Answer>>,
    registration: Option<PathBuf>,
}

impl ContextServer {
    pub fn bind(path: PathBuf, wake: Wake) -> io::Result<Self> {
        let dir = path
            .parent()
            .ok_or_else(|| io::Error::other("socket needs a private directory"))?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other("MCP directory must be private (0700)"));
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let owner = std::fs::metadata(&path)?.uid();
        let ticket = new_ticket()?;
        let (tx, receiver) = mpsc::channel(64);
        let secret = ticket.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let next = Arc::new(AtomicU64::new(1));
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        if stream.peer_cred().is_ok_and(|c| c.uid() == owner) && connections.len() < 32 {
                            connections.spawn(serve(stream, secret.clone(), tx.clone(), wake.clone(), next.clone()));
                        }
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            listener: McpListener { path, ticket, task },
            receiver,
            pending: BTreeMap::new(),
            registration: None,
        })
    }

    pub fn start(wake: Wake) -> io::Result<Self> {
        let dir = registry_dir().join(format!("{}-{}", std::process::id(), new_ticket()?));
        let mut server = Self::bind(dir.join("context.sock"), wake)?;
        let registration = dir.join("endpoint.json");
        let endpoint = Endpoint {
            socket: server.listener.path.clone(),
            ticket: server.listener.ticket.clone(),
            workspace: std::env::current_dir()?,
        };
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&registration)?;
        file.write_all(&serde_json::to_vec(&endpoint).map_err(io::Error::other)?)?;
        server.registration = Some(registration);
        Ok(server)
    }
    pub fn tool_server(&self, executable: &Path) -> ToolServer {
        let mut server = self.listener.tool_server(executable);
        server.tools = TOOLS.iter().map(|s| (*s).to_owned()).collect();
        server
    }
    pub fn calls(&mut self) -> Vec<Call> {
        self.pending.retain(|_, tx| !tx.is_closed());
        let mut calls = vec![];
        while let Ok(p) = self.receiver.try_recv() {
            if p.answer.is_closed() {
                continue;
            }
            self.pending.insert(p.call.id, p.answer);
            calls.push(p.call);
        }
        calls
    }
    pub fn answer(&mut self, id: u64, answer: Answer) {
        if let Some(tx) = self.pending.remove(&id) {
            let _ = tx.send(answer);
        }
    }
}
impl Drop for ContextServer {
    fn drop(&mut self) {
        if let Some(path) = &self.registration {
            let _ = std::fs::remove_file(path);
        }
        self.listener.task.abort();
        let _ = std::fs::remove_file(&self.listener.path);
        if let Some(path) = &self.registration
            && let Some(dir) = path.parent()
        {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

fn registry_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| radar_core::runner::config_home().join("run"))
        .join("radar-mcp")
}

pub fn discover(workspace: Option<&Path>) -> io::Result<(PathBuf, String)> {
    let requested = workspace.map(std::fs::canonicalize).transpose()?;
    let mut found = vec![];
    if let Ok(entries) = std::fs::read_dir(registry_dir()) {
        for entry in entries.flatten() {
            let path = entry.path().join("endpoint.json");
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(endpoint) = serde_json::from_slice::<Endpoint>(&bytes) else {
                continue;
            };
            if requested.as_ref().is_some_and(|w| {
                std::fs::canonicalize(&endpoint.workspace).ok().as_ref() != Some(w)
            }) {
                continue;
            }
            // A stale endpoint is ignored without deleting another process's files.
            if std::os::unix::net::UnixStream::connect(&endpoint.socket).is_ok() {
                found.push(endpoint);
            }
        }
    }
    match found.len() {
        1 => {
            let e = found.remove(0);
            Ok((e.socket, e.ticket))
        }
        0 => Err(io::Error::other(
            "no running radar context server; start radar in that workspace first",
        )),
        _ => Err(io::Error::other(
            "several radar hosts match; select one with --workspace or --socket and --ticket",
        )),
    }
}

pub fn specs() -> Value {
    let descriptions = [
        "Find cached work across radar's configured sources. Accepts exact URLs or text, optional kind/project/assignee filters. Returns scoped refs and a selection_ref; coverage says what was searched. Use context next.",
        "Assemble linked work from roots (URLs or radar refs) or a search selection_ref: requirements, changes, discussions, checkouts, relationships and source gaps. Read full sections with read. Snapshots expire after an hour or cache eviction; cursors continue the captured snapshot.",
        "Read full cached source sections for up to 32 refs together. Use snapshot_id to read the exact revision returned by context. Optional sections select named sections. Follow next_cursor for the rest; truncated provider caches are marked explicitly.",
        "Compare a prior snapshot_id with the current cached connected work. Returns changed and unavailable refs and a new snapshot. This reads current caches, not a guaranteed fresh remote view.",
    ];
    let mut specs = json!(TOOLS.iter().zip(descriptions).map(|(name, description)| json!({
        "name":name,"description":description,
        "inputSchema":{"type":"object","additionalProperties":false,"properties":{
            "query":{"type":"string"},"roots":{"type":"array","items":{"type":"string"}},
            "selection_ref":{"type":"string"},"refs":{"type":"array","items":{"type":"string"},"maxItems":32},
            "sections":{"type":"array","items":{"type":"string"}},"kind":{"type":"string"},"project":{"type":"string"},"assignee":{"type":"string"},
            "snapshot_id":{"type":"string"},"cursor":{"type":"string"},"max_bytes":{"type":"integer","minimum":1024,"maximum":262144},
            "depth":{"type":"integer","minimum":0,"maximum":4}
        }},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}
    })).collect::<Vec<_>>());
    for tool in specs.as_array_mut().expect("tool specs") {
        let accepted = radar_core::context::arguments(tool["name"].as_str().expect("tool name"));
        tool["inputSchema"]["properties"]
            .as_object_mut()
            .expect("properties")
            .retain(|name, _| accepted.contains(&name.as_str()));
    }
    specs
}

fn clean(value: &mut Value) {
    match value {
        Value::String(s) => *s = crate::brief::redact_secrets(s).0,
        Value::Array(a) => a.iter_mut().for_each(clean),
        Value::Object(o) => o.values_mut().for_each(clean),
        _ => {}
    }
}
fn result(id: Value, answer: Answer) -> Value {
    match answer {
        Ok(mut data) => {
            clean(&mut data);
            json!({"jsonrpc":"2.0","id":id,"result":{"structuredContent":data,"content":[{"type":"text","text":serde_json::to_string(&data).unwrap_or_default()}]}})
        }
        Err(message) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":crate::brief::redact_secrets(&message).0}]}})
        }
    }
}

async fn serve(
    stream: UnixStream,
    ticket: String,
    tx: mpsc::Sender<Pending>,
    wake: Wake,
    next: Arc<AtomicU64>,
) -> io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    let mut line = vec![];
    let first = tokio::time::timeout(Duration::from_secs(5), read_line(&mut rd, &mut line)).await;
    if !matches!(first, Ok(Ok(Some(())))) {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
    if !value["radar_ticket"]
        .as_str()
        .is_some_and(|t| super::same(t, &ticket))
    {
        return Ok(());
    }
    let (out, mut replies) = mpsc::channel::<Value>(32);
    let mut tasks = JoinSet::new();
    let (input, mut incoming) = mpsc::channel(32);
    tasks.spawn(async move {
        while let Ok(Some(())) = read_line(&mut rd, &mut line).await {
            if input.send(line.clone()).await.is_err() {
                break;
            }
        }
    });
    let mut running: BTreeMap<String, tokio::task::AbortHandle> = BTreeMap::new();
    loop {
        tokio::select! {
            got = incoming.recv() => {
                let Some(line) = got else { break; };
                let msg: Value = match serde_json::from_slice(&line) {
                    Ok(v) => v,
                    Err(_) => { out.try_send(super::error(&Value::Null, -32700, "not JSON")).map_err(io::Error::other)?; continue; }
                };
                let method = msg["method"].as_str().unwrap_or_default();
                if method == "notifications/cancelled" {
                    if let Some(task) = running.remove(&msg["params"]["requestId"].to_string()) { task.abort(); }
                    continue;
                }
                let Some(id) = msg.get("id").cloned() else { continue };
                let reply = match method {
                    "tools/list" => Some(super::result(&id, json!({"tools":specs()}))),
                    "tools/call" => {
                        let name = msg["params"]["name"].as_str().unwrap_or_default().to_owned();
                        running.retain(|_, h| !h.is_finished());
                        if !TOOLS.contains(&name.as_str()) { Some(super::error(&id, -32602, "unknown radar tool")) }
                        else if running.len() >= 8 { Some(result(id, Err("too many concurrent reads; retry when a pending read finishes".into()))) }
                        else {
                            let key = id.to_string();
                            if let std::collections::btree_map::Entry::Vacant(entry) = running.entry(key) {
                                let (answer, receive) = oneshot::channel();
                                let call = Call { id: next.fetch_add(1, Ordering::Relaxed), tool: name, arguments: msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({})) };
                                if tx.try_send(Pending { call, answer }).is_err() { Some(result(id, Err("radar is busy; retry shortly".into()))) }
                                else {
                                    wake();
                                    let out = out.clone();
                                    let task = tasks.spawn(async move {
                                        let answer = match tokio::time::timeout(Duration::from_secs(30), receive).await {
                                            Ok(Ok(v)) => v,
                                            _ => Err("radar did not answer in time; it may be paused or closing".into()),
                                        };
                                        let _ = out.send(result(id, answer)).await;
                                    });
                                    entry.insert(task);
                                    None
                                }
                            } else { Some(super::error(&id, -32600, "request ID is already running")) }
                        }
                    }
                    _ => super::answer(&msg),
                };
                if let Some(reply) = reply { out.try_send(reply).map_err(io::Error::other)?; }
            }
            Some(reply) = replies.recv() => {
                let mut bytes = serde_json::to_vec(&reply).map_err(io::Error::other)?;
                if bytes.len() > super::MAX_LINE { bytes = serde_json::to_vec(&result(reply["id"].clone(), Err("result exceeds transport limit; lower max_bytes".into()))).map_err(io::Error::other)?; }
                bytes.push(b'\n'); wr.write_all(&bytes).await?;
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
