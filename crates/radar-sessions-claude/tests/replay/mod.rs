//! A [`Spawner`] that serves a recorded sidecar run (tests/fixtures/live.ndjson), so adapter
//! tests run with no node, no Claude Code and no clock.
//!
//! Each `spawn()` plays the next recorded process: it says `ready`, then, for every request the
//! adapter writes, finds the first unserved recorded request with the same `op` and answers with
//! the recorded lines that followed it (the reply gets the live request's id).

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use radar_sessions::BoxFuture;
use radar_sessions_claude::{Spawned, Spawner};
use serde_json::{Value, json};
use tokio::sync::mpsc;

pub const RECORDING: &str = include_str!("../fixtures/live.ndjson");

/// The recording split into its processes: each starts at a `ready` line.
pub fn processes() -> Vec<Vec<(bool, Value)>> {
    let mut procs: Vec<Vec<(bool, Value)>> = vec![];
    for l in RECORDING.lines().filter(|l| !l.is_empty()) {
        let v: Value = serde_json::from_str(l).unwrap();
        let incoming = v["dir"] == "in";
        if v["line"]["type"] == "ready" {
            procs.push(vec![]);
        }
        procs.last_mut().unwrap().push((incoming, v["line"].clone()));
    }
    procs
}

pub struct Replay {
    procs: Mutex<VecDeque<Vec<(bool, Value)>>>,
    /// Stop answering after this many requests, closing the sidecar's stdout: a crash.
    pub die_after: Option<usize>,
    /// Play the first process again for every spawn.
    pub repeat: bool,
    pub requests: Arc<Mutex<Vec<Value>>>,
}

impl Replay {
    /// Plays the recorded processes from the `first`th on, one per spawn.
    pub fn from(first: usize) -> Self {
        Self { procs: Mutex::new(processes().into_iter().skip(first).collect()), die_after: None, repeat: false, requests: Arc::default() }
    }
    pub fn dying_after(mut self, n: usize) -> Self {
        self.die_after = Some(n);
        self
    }
    pub fn repeating(mut self) -> Self {
        self.repeat = true;
        self
    }
}

impl Spawner for Replay {
    fn spawn(&self) -> BoxFuture<'_, Result<Spawned>> {
        Box::pin(async move {
            let script = {
                let mut procs = self.procs.lock().unwrap();
                if self.repeat { procs.front().cloned() } else { procs.pop_front() }
            }
            .ok_or_else(|| anyhow::anyhow!("replay: no more recorded processes"))?;
            let (line_tx, lines) = mpsc::unbounded_channel::<String>();
            let (writer, mut wrx) = mpsc::unbounded_channel::<String>();
            let die_after = self.die_after;
            let seen = Arc::clone(&self.requests);
            tokio::spawn(async move {
                let mut script: VecDeque<(bool, Value)> = script.into();
                // `ready`
                let (_, ready) = script.pop_front().unwrap();
                let _ = line_tx.send(ready.to_string());
                let mut handled = 0usize;
                while let Some(req) = wrx.recv().await {
                    if die_after.is_some_and(|n| handled >= n) {
                        return; // dropping line_tx closes the sidecar's stdout
                    }
                    let req: Value = serde_json::from_str(&req).unwrap();
                    seen.lock().unwrap().push(req.clone());
                    handled += 1;
                    // The first recorded request with this op that has not been served yet.
                    let Some(at) = script.iter().position(|(incoming, r)| *incoming && r["op"] == req["op"]) else {
                        let _ = line_tx.send(json!({"type":"reply","id":req["id"],"ok":false,"error":format!("replay: no recorded {}", req["op"])}).to_string());
                        continue;
                    };
                    script.remove(at);
                    while script.get(at).is_some_and(|(incoming, _)| !incoming) {
                        let (_, mut out) = script.remove(at).unwrap();
                        if out["type"] == "reply" {
                            out["id"] = req["id"].clone();
                        }
                        let _ = line_tx.send(out.to_string());
                    }
                }
            });
            Ok(Spawned { lines, writer, stderr_tail: Arc::default() })
        })
    }
}
