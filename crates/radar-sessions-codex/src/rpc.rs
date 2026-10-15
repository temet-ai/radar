//! JSON-RPC 2.0 over lines, the way `codex app-server` speaks it (no
//! `"jsonrpc"` member). One [`Conn`] is one app-server process.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

type Reply = Result<Value, String>;

pub struct Conn {
    /// Which process this is; ids from a dead process mean nothing to a new one.
    pub generation: u64,
    tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
}

impl Conn {
    pub fn new(generation: u64, tx: mpsc::UnboundedSender<String>) -> Self {
        Self {
            generation,
            tx: Mutex::new(Some(tx)),
            next: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn send(&self, msg: Value) -> bool {
        match self.tx.lock().unwrap().as_ref() {
            Some(tx) => tx.send(msg.to_string()).is_ok(),
            None => false,
        }
    }

    /// Ask and wait. Fails when the process is gone, answers with an error,
    /// or stays silent for `timeout`.
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if !self.send(json!({ "method": method, "id": id, "params": params })) {
            self.pending.lock().unwrap().remove(&id);
            anyhow::bail!("Codex is not running");
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => anyhow::bail!("Codex refused `{method}`: {e}"),
            Ok(Err(_)) => anyhow::bail!("Codex exited during `{method}`"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                anyhow::bail!("Codex did not answer `{method}` in time")
            }
        }
    }

    pub fn notify(&self, method: &str) {
        self.send(json!({ "method": method }));
    }

    /// Answer a request the server made.
    pub fn respond(&self, id: &Value, result: Value) -> bool {
        self.send(json!({ "id": id, "result": result }))
    }

    pub fn respond_error(&self, id: &Value, message: &str) -> bool {
        self.send(json!({ "id": id, "error": { "code": -32601, "message": message } }))
    }

    /// A response from the server to one of our requests.
    pub fn resolve(&self, id: u64, reply: Reply) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(reply);
        }
    }

    /// The process is gone: wake everyone waiting, refuse everything after.
    pub fn close(&self) {
        self.tx.lock().unwrap().take();
        self.pending.lock().unwrap().clear();
    }
}

/// What one line from the server is.
pub enum Incoming {
    Response {
        id: u64,
        reply: Reply,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn parse(line: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(line).ok()?;
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match (v.get("method").and_then(Value::as_str), v.get("id")) {
        (Some(m), Some(id)) => Some(Incoming::Request {
            id: id.clone(),
            method: m.into(),
            params,
        }),
        (Some(m), None) => Some(Incoming::Notification {
            method: m.into(),
            params,
        }),
        (None, Some(id)) => {
            let reply = match v.get("error") {
                Some(e) => Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string()),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Response {
                id: id.as_u64()?,
                reply,
            })
        }
        _ => None,
    }
}
