//! A fake `codex app-server` for tests: plays back a recorded (or scripted)
//! conversation. No process, no clock.
//!
//! A script is a list of segments. A segment is one message the client is
//! expected to send, and the lines the server says after it. The client's
//! messages are matched by method (requests, notifications) or by id
//! (answers to the server's own requests), so the order the adapter makes
//! independent calls in does not matter.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use radar_sessions::BoxFuture;
use radar_sessions::adapter::ProviderSetup;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::wire::{Spawner, Wire};

#[derive(Clone, Debug)]
pub enum Out {
    Request(String),
    Notify(String),
    /// The client answers a request the server made, by the server's id.
    Response(Value),
    /// The same, when the answer's `decision` must be this one.
    Answer(Value, String),
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub out: Out,
    /// What the server says next. A line `{"$close":true}` ends the process.
    pub ins: Vec<Value>,
}

/// `{"$close": true}`: the process exits here.
pub fn close() -> Value {
    json!({ "$close": true })
}

pub fn request(method: &str, ins: Vec<Value>) -> Seg {
    Seg {
        out: Out::Request(method.into()),
        ins,
    }
}

pub fn notify(method: &str) -> Seg {
    Seg {
        out: Out::Notify(method.into()),
        ins: vec![],
    }
}

pub fn response(id: Value, ins: Vec<Value>) -> Seg {
    Seg {
        out: Out::Response(id),
        ins,
    }
}

/// The result line for a request; the id is filled in when it is played.
pub fn answer(id: Value, decision: &str, ins: Vec<Value>) -> Seg {
    Seg {
        out: Out::Answer(id, decision.into()),
        ins,
    }
}

pub fn result(v: Value) -> Value {
    json!({ "id": null, "result": v, "$answer": true })
}

pub fn note(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

pub fn server_request(id: i64, method: &str, params: Value) -> Value {
    json!({ "method": method, "id": id, "params": params })
}

/// Segments from a recording: `{"dir": "out"|"in", "msg": ...}` per line.
pub fn load(jsonl: &str) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).expect("recording line");
        let msg = row["msg"].clone();
        if row["dir"] == "out" {
            let out = match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
                (Some(m), Some(_)) => Out::Request(m.into()),
                (Some(m), None) => Out::Notify(m.into()),
                (None, Some(id)) => Out::Response(id.clone()),
                _ => panic!("odd out line {line}"),
            };
            segs.push(Seg { out, ins: vec![] });
        } else {
            let mut m = msg;
            // A response in the recording carries the recorded request id;
            // mark it to be re-addressed to the client's own.
            if m.get("method").is_none() && m.get("id").is_some() {
                m["$answer"] = json!(true);
            }
            segs.last_mut()
                .expect("server line before any client line")
                .ins
                .push(m);
        }
    }
    segs
}

#[derive(Default)]
pub struct Log {
    /// Every message the client sent, in order.
    pub sent: Vec<Value>,
    /// Messages that matched no segment.
    pub unexpected: Vec<Value>,
}

pub struct ReplaySpawner {
    /// One script per process started.
    scripts: Mutex<VecDeque<Vec<Seg>>>,
    pub log: Arc<Mutex<Log>>,
    pub version: Option<String>,
    pub spawns: Mutex<usize>,
    /// Every `configure` the adapter passed on, in order.
    pub configured: Mutex<Vec<ProviderSetup>>,
    /// While true, starting the app-server fails (a lost connection that has
    /// not come back).
    pub down: std::sync::atomic::AtomicBool,
    /// The running process's pipe to the client, and its off switch.
    current: Mutex<Option<(mpsc::UnboundedSender<String>, mpsc::UnboundedSender<()>)>>,
}

impl ReplaySpawner {
    pub fn new(scripts: Vec<Vec<Seg>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            log: Arc::default(),
            version: Some("0.158.0".into()),
            spawns: Mutex::new(0),
            configured: Mutex::default(),
            down: Default::default(),
            current: Mutex::new(None),
        })
    }

    /// A machine with no `codex` installed.
    pub fn not_installed() -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(VecDeque::new()),
            log: Arc::default(),
            version: None,
            spawns: Mutex::new(0),
            configured: Mutex::default(),
            down: Default::default(),
            current: Mutex::new(None),
        })
    }

    pub fn push_script(&self, script: Vec<Seg>) {
        self.scripts.lock().unwrap().push_back(script);
    }

    /// The server says something on its own.
    pub fn inject(&self, lines: Vec<Value>) {
        if let Some((tx, _)) = self.current.lock().unwrap().as_ref() {
            for l in lines {
                let _ = tx.send(l.to_string());
            }
        }
    }

    /// The process dies.
    pub fn kill(&self) {
        if let Some((_, off)) = self.current.lock().unwrap().take() {
            let _ = off.send(());
        }
    }

    /// The `params` of the first client request named `method`.
    pub fn sent(&self, method: &str) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m.get("method").and_then(Value::as_str) == Some(method))
            .map(|m| m.get("params").cloned().unwrap_or(Value::Null))
            .collect()
    }

    pub fn answers(&self) -> Vec<Value> {
        self.log
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m.get("method").is_none())
            .cloned()
            .collect()
    }
}

/// `Spawner` is implemented for an `Arc` so a test keeps a handle on the log.
impl Spawner for Arc<ReplaySpawner> {
    fn spawn(&self) -> BoxFuture<'_, anyhow::Result<Wire>> {
        Box::pin(async move {
            if self.down.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("cannot start `codex app-server`: the test has it down");
            }
            *self.spawns.lock().unwrap() += 1;
            let mut segs: Vec<Option<Seg>> = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| {
                    anyhow::anyhow!("cannot start `codex app-server`: no more scripted processes")
                })?
                .into_iter()
                .map(Some)
                .collect();
            let log = Arc::clone(&self.log);
            let me = Arc::clone(self);
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
            let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
            let (off_tx, mut off_rx) = mpsc::unbounded_channel::<()>();
            *self.current.lock().unwrap() = Some((in_tx.clone(), off_tx));
            tokio::spawn(async move {
                loop {
                    let line = tokio::select! {
                        l = out_rx.recv() => match l { Some(l) => l, None => break },
                        _ = off_rx.recv() => return,
                    };
                    let msg: Value = serde_json::from_str(&line).expect("client sent json");
                    log.lock().unwrap().sent.push(msg.clone());
                    let method = msg.get("method").and_then(Value::as_str);
                    let id = msg.get("id").cloned();
                    let hit = segs.iter().position(|s| {
                        match (&s.as_ref().map(|s| &s.out), method, &id) {
                            (Some(Out::Request(m)), Some(x), Some(_)) => m == x,
                            (Some(Out::Notify(m)), Some(x), None) => m == x,
                            (Some(Out::Response(want)), None, Some(got)) => want == got,
                            (Some(Out::Answer(want, decision)), None, Some(got)) => {
                                want == got && msg["result"]["decision"] == json!(decision)
                            }
                            _ => false,
                        }
                    });
                    let Some(i) = hit else {
                        // The adapter asks for the session's skills on its own; a script that
                        // does not care gets a machine with none.
                        if method == Some("skills/list") && let Some(id) = &id {
                            let _ = in_tx.send(json!({ "id": id, "result": { "data": [] } }).to_string());
                            continue;
                        }
                        log.lock().unwrap().unexpected.push(msg);
                        continue;
                    };
                    let seg = segs[i].take().unwrap();
                    for mut m in seg.ins {
                        if m.get("$close").is_some() {
                            me.current.lock().unwrap().take();
                            return; // drops in_tx: the process is gone
                        }
                        if m.get("$answer").is_some() {
                            m.as_object_mut().unwrap().remove("$answer");
                            m["id"] = id.clone().unwrap_or(Value::Null);
                        }
                        let _ = in_tx.send(m.to_string());
                    }
                }
            });
            Ok(Wire {
                tx: out_tx,
                rx: in_rx,
            })
        })
    }

    fn version(&self) -> BoxFuture<'_, Option<String>> {
        let v = self.version.clone();
        Box::pin(async move { v })
    }

    fn configure(&self, setup: &ProviderSetup) {
        self.configured.lock().unwrap().push(setup.clone());
    }
}
