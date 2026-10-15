//! The supervised link to the Node sidecar: newline-delimited JSON over its stdio.
//!
//! A [`Spawner`] starts one sidecar and hands back its two line channels, so the same code
//! runs against the real `node` ([`ProcessSpawner`]) and against a replayed recording in tests.
//! The link starts the sidecar on first use, checks its protocol, restarts it after it exits
//! (at most [`MAX_RESTARTS`] starts in [`RESTART_WINDOW`] seconds, counted on the injected
//! clock), and tells the part when the connection was lost and when it is back.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use radar_sessions::BoxFuture;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

/// The sidecar protocol this crate speaks (sidecars/claude-agent/src/server.js `PROTOCOL`).
pub const PROTOCOL: u64 = 1;
/// At most this many sidecar starts within [`RESTART_WINDOW`] seconds.
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW: i64 = 60;
/// How long a started sidecar has to say `ready`, and how long a request has to be answered.
pub const READY_WITHIN: Duration = Duration::from_secs(15);
pub const REPLY_WITHIN: Duration = Duration::from_secs(60);

/// One running sidecar: lines in, lines out.
pub struct Spawned {
    /// Lines the sidecar wrote to stdout. The channel closes when it exits.
    pub lines: mpsc::UnboundedReceiver<String>,
    /// Lines to write to its stdin.
    pub writer: mpsc::UnboundedSender<String>,
    /// What the sidecar said on stderr last, for the error shown when it dies.
    pub stderr_tail: Arc<StdMutex<VecDeque<String>>>,
}

pub trait Spawner: Send + Sync {
    fn spawn(&self) -> BoxFuture<'_, Result<Spawned>>;
}

/// What the link tells the adapter, besides replies.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkMsg {
    /// An unprompted `{"type":"event",..}` line.
    Event(Value),
    /// An unprompted `{"type":"commands","session",..,"commands":[..]}` line: the slash
    /// commands one session accepts, the whole list. State, not history.
    Commands(Value),
    Disconnected(String),
    Reconnected,
}

type Pending = Arc<StdMutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

struct Conn {
    generation: u64,
    writer: mpsc::UnboundedSender<String>,
}

struct State {
    conn: Option<Conn>,
    generation: u64,
    starts: VecDeque<i64>,
    ever_connected: bool,
}

pub struct Link {
    spawner: Box<dyn Spawner>,
    state: Mutex<State>,
    pending: Pending,
    next_id: AtomicU64,
    msgs: mpsc::UnboundedSender<LinkMsg>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    sdk_version: StdMutex<Option<String>>,
}

impl Link {
    pub fn new(
        spawner: Box<dyn Spawner>,
        now: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<LinkMsg>) {
        let (msgs, rx) = mpsc::unbounded_channel();
        let link = Arc::new(Self {
            spawner,
            state: Mutex::new(State { conn: None, generation: 0, starts: VecDeque::new(), ever_connected: false }),
            pending: Arc::default(),
            next_id: AtomicU64::new(1),
            msgs,
            now,
            sdk_version: StdMutex::default(),
        });
        (link, rx)
    }

    /// The Agent SDK version the running sidecar reported when it started.
    pub fn sdk_version(&self) -> Option<String> {
        self.sdk_version.lock().unwrap().clone()
    }

    /// Send `{id, op, ..args}` and wait for its reply's `result`.
    pub async fn request(self: &Arc<Self>, op: &str, args: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut req = json!({ "id": id, "op": op });
        if let (Some(r), Some(a)) = (req.as_object_mut(), args.as_object()) {
            r.extend(a.clone());
        }
        let line = req.to_string();
        if let Err(e) = self.write(line).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(REPLY_WITHIN, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("the Claude Code sidecar exited before it answered {op}")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(anyhow!("the Claude Code sidecar did not answer {op} in time"))
            }
        }
    }

    /// Whether a sidecar is running and speaks our protocol (starts one when none is).
    pub async fn health(self: &Arc<Self>) -> Result<Value> {
        self.request("hello", json!({})).await
    }

    async fn write(self: &Arc<Self>, line: String) -> Result<()> {
        let mut st = self.state.lock().await;
        if st.conn.is_none() {
            self.connect(&mut st).await?;
        }
        let conn = st.conn.as_ref().expect("connected");
        conn.writer.send(line).map_err(|_| anyhow!("the Claude Code sidecar is gone"))
    }

    // Boxed by hand: `connect` spawns a task that can call `lost`, which spawns one that calls
    // `connect`; an `async fn` would make that a cycle of opaque types.
    fn connect<'a>(self: &'a Arc<Self>, st: &'a mut State) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.connect_inner(st))
    }

    async fn connect_inner(self: &Arc<Self>, st: &mut State) -> Result<()> {
        let now = (self.now)();
        while st.starts.front().is_some_and(|t| now - *t >= RESTART_WINDOW) {
            st.starts.pop_front();
        }
        if st.starts.len() >= MAX_RESTARTS {
            bail!("the Claude Code sidecar keeps exiting ({MAX_RESTARTS} starts in {RESTART_WINDOW}s); not starting it again for now");
        }
        let Spawned { mut lines, writer, stderr_tail } = self.spawner.spawn().await?;
        // The first line is `ready`; then `hello` confirms the protocol before anything else is sent.
        let ready = tokio::time::timeout(READY_WITHIN, lines.recv()).await;
        let first = match ready {
            Ok(Some(l)) => l,
            Ok(None) => bail!("the Claude Code sidecar exited at start{}", tail(&stderr_tail)),
            Err(_) => bail!("the Claude Code sidecar did not start in time"),
        };
        let v: Value = serde_json::from_str(&first).map_err(|e| anyhow!("sidecar said something unreadable: {e}"))?;
        if v["type"] != "ready" {
            bail!("the Claude Code sidecar did not start with ready");
        }
        if v["protocol"].as_u64() != Some(PROTOCOL) {
            bail!("sidecar speaks protocol {}, radar expects {PROTOCOL}", v["protocol"]);
        }
        *self.sdk_version.lock().unwrap() = v["sdk_version"].as_str().map(str::to_string);
        // Only a sidecar that started counts: a spawn that fails is retried with back-off.
        st.starts.push_back(now);
        st.generation += 1;
        let generation = st.generation;
        st.conn = Some(Conn { generation, writer });
        let reconnect = st.ever_connected;
        st.ever_connected = true;

        // Weak: dropping the link closes the sidecar's stdin, and it exits.
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(line) = lines.recv().await {
                let Some(link) = me.upgrade() else { return };
                link.route(&line);
            }
            let why = format!("the Claude Code sidecar exited{}", tail(&stderr_tail));
            if let Some(link) = me.upgrade() {
                link.lost(generation, why).await;
            }
        });
        if reconnect {
            let _ = self.msgs.send(LinkMsg::Reconnected);
        }
        Ok(())
    }

    fn route(&self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        match v["type"].as_str() {
            Some("reply") => {
                let Some(id) = v["id"].as_u64() else { return };
                if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                    let r = if v["ok"].as_bool() == Some(true) {
                        Ok(v["result"].clone())
                    } else {
                        Err(v["error"].as_str().unwrap_or("sidecar error").to_string())
                    };
                    let _ = tx.send(r);
                }
            }
            Some("event") => {
                let _ = self.msgs.send(LinkMsg::Event(v));
            }
            Some("commands") => {
                let _ = self.msgs.send(LinkMsg::Commands(v));
            }
            _ => {}
        }
    }

    async fn lost(self: &Arc<Self>, generation: u64, why: String) {
        let mut st = self.state.lock().await;
        if st.conn.as_ref().is_some_and(|c| c.generation == generation) {
            st.conn = None;
        } else {
            return;
        }
        drop(st);
        // Dropping the senders ends every waiting request with "exited before it answered".
        self.pending.lock().unwrap().clear();
        let _ = self.msgs.send(LinkMsg::Disconnected(why));
        // Start it again without waiting for the next request: the part resumes its sessions
        // from their cursors when it hears `Reconnected`.
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut delay = 1;
            loop {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                let Some(link) = me.upgrade() else { return };
                let mut st = link.state.lock().await;
                if st.conn.is_some() || link.connect(&mut st).await.is_ok() {
                    return;
                }
                delay = (delay * 2).min(30);
            }
        });
    }
}

fn tail(t: &StdMutex<VecDeque<String>>) -> String {
    let t = t.lock().unwrap();
    if t.is_empty() { String::new() } else { format!(": {}", t.iter().cloned().collect::<Vec<_>>().join(" / ")) }
}

/// Starts the sidecar (`<program> <args>`) with piped stdio. The sidecar is killed when the
/// link is dropped and exits by itself when its stdin closes.
#[derive(Debug, PartialEq)]
pub struct ProcessSpawner {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

impl ProcessSpawner {
    /// `node` and the sidecar script `RADAR_CLAUDE_SIDECAR` names; else radar's runtime
    /// (`RADAR_RUNTIME`, the executable the radar binary carries) as `<runtime> claude-agent`, with no
    /// node; else `node` on `PATH` and the sidecar in
    /// `<exe dir>/../share/radar/sidecars/claude-agent/src/main.js` or the checkout this crate
    /// was built in.
    pub fn locate(path_var: Option<&str>, env: &dyn Fn(&str) -> Option<String>, exe: Option<&Path>) -> Result<Self> {
        let named = env("RADAR_CLAUDE_SIDECAR").filter(|s| !s.is_empty());
        if named.is_none()
            && let Some(runtime) = env("RADAR_RUNTIME").filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_file())
        {
            return Ok(Self { program: runtime, args: vec!["claude-agent".into()] });
        }
        let node = std::env::split_paths(path_var.unwrap_or_default())
            .map(|d| d.join("node"))
            .find(|p| p.is_file())
            .ok_or_else(|| anyhow!("`node` is not on PATH; the Claude Code adapter needs it"))?;
        let mut candidates: Vec<PathBuf> = vec![];
        if let Some(s) = named {
            candidates.push(s.into());
        }
        if let Some(dir) = exe.and_then(|e| e.parent()) {
            candidates.push(dir.join("../share/radar/sidecars/claude-agent/src/main.js"));
        }
        candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/claude-agent/src/main.js"));
        let script = candidates
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| anyhow!("the Claude Code sidecar (sidecars/claude-agent) was not found"))?;
        Ok(Self { program: node, args: vec![script.into()] })
    }
}

impl Spawner for ProcessSpawner {
    fn spawn(&self) -> BoxFuture<'_, Result<Spawned>> {
        Box::pin(async move {
            let mut child = tokio::process::Command::new(&self.program)
                .args(&self.args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| anyhow!("cannot start {}: {e}", self.program.display()))?;
            let mut stdin = child.stdin.take().expect("piped");
            let stdout = child.stdout.take().expect("piped");
            let stderr = child.stderr.take().expect("piped");
            let (line_tx, lines) = mpsc::unbounded_channel();
            let (writer, mut wrx) = mpsc::unbounded_channel::<String>();
            let stderr_tail: Arc<StdMutex<VecDeque<String>>> = Arc::default();

            tokio::spawn(async move {
                let mut r = BufReader::new(stdout).lines();
                while let Ok(Some(l)) = r.next_line().await {
                    if line_tx.send(l).is_err() {
                        break;
                    }
                }
                // Keep `child` alive until stdout closes, then reap it.
                let _ = child.wait().await;
            });
            let tail = Arc::clone(&stderr_tail);
            tokio::spawn(async move {
                let mut r = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = r.next_line().await {
                    let mut t = tail.lock().unwrap();
                    if t.len() >= 5 {
                        t.pop_front();
                    }
                    t.push_back(l.chars().take(200).collect());
                }
            });
            tokio::spawn(async move {
                while let Some(l) = wrx.recv().await {
                    if stdin.write_all(l.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
                // The writer was dropped: closing stdin tells the sidecar to end its sessions and exit.
            });
            Ok(Spawned { lines, writer, stderr_tail })
        })
    }
}

#[cfg(test)]
mod locate_tests {
    use super::*;

    #[test]
    fn the_runtime_runs_the_sidecar_unless_a_script_is_named() {
        let dir = std::env::temp_dir().join(format!("radar-claude-locate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (runtime, script) = (dir.join("radar-js"), dir.join("main.js"));
        for f in [&runtime, &script, &dir.join("node")] {
            std::fs::write(f, "").unwrap();
        }
        let env = |named: bool| {
            let (runtime, script) = (runtime.display().to_string(), script.display().to_string());
            move |k: &str| match k {
                "RADAR_RUNTIME" => Some(runtime.clone()),
                "RADAR_CLAUDE_SIDECAR" if named => Some(script.clone()),
                _ => None,
            }
        };
        let path = dir.display().to_string();
        assert_eq!(
            ProcessSpawner::locate(None, &env(false), None).unwrap(),
            ProcessSpawner { program: runtime.clone(), args: vec!["claude-agent".into()] }
        );
        assert_eq!(
            ProcessSpawner::locate(Some(&path), &env(true), None).unwrap(),
            ProcessSpawner { program: dir.join("node"), args: vec![script.clone().into()] }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
