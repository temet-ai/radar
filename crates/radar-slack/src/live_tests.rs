//! A read-only live measurement against a real workspace, ignored by default.
//!
//! `RADAR_SLACK_LIVE_CONFIG=<toml with a [slack] table>` (the token `env:NAME`),
//! then `cargo test -p radar-slack --lib -- --ignored --nocapture live_cold_start`.
//! It only reads: no post, reaction or mark is ever sent. It prints, for a
//! cold start, a refresh one refresh later and a restart from the saved
//! cache, the time to quiet and the bytes and requests per Slack method.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use radar_core::Plugin;
use radar_core::plugin::{Effect, Msg, Task};

use crate::SlackPlugin;
use crate::client::{Fut, Http, Reply, Request, Transport};

struct Counting {
    http: Http,
    seen: Mutex<Vec<(&'static str, usize)>>,
}

impl Transport for Counting {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, anyhow::Result<Reply>> {
        Box::pin(async move {
            assert!(!req.write, "the live measurement never writes");
            let reply = self.http.send(req).await?;
            self.seen
                .lock()
                .unwrap()
                .push((req.method, reply.body.len()));
            Ok(reply)
        })
    }
}

fn tasks(fx: Vec<Effect>) -> Vec<Task> {
    fx.into_iter()
        .filter_map(|e| match e {
            Effect::Spawn(task) | Effect::SpawnAction { task, .. } => Some(task),
            _ => None,
        })
        .collect()
}

/// Run every task as the host does, concurrently, feeding each answer back.
async fn drive(p: &mut SlackPlugin, first: Vec<Effect>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Msg>();
    let mut open = 0usize;
    let launch = |tasks: Vec<Task>, open: &mut usize| {
        for task in tasks {
            *open += 1;
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(task.await);
            });
        }
    };
    launch(tasks(first), &mut open);
    while open > 0 {
        let msg = rx.recv().await.expect("a task answer");
        open -= 1;
        let fx = p.on_msg(msg);
        launch(tasks(fx), &mut open);
    }
}

fn report(what: &str, took: std::time::Duration, counting: &Counting) {
    let mut by: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (method, bytes) in counting.seen.lock().unwrap().drain(..) {
        let e = by.entry(method).or_default();
        e.0 += 1;
        e.1 += bytes;
    }
    let total: usize = by.values().map(|v| v.1).sum();
    let calls: usize = by.values().map(|v| v.0).sum();
    println!(
        "== {what}: {:.1}s, {calls} requests, {total} bytes",
        took.as_secs_f64()
    );
    for (method, (n, bytes)) in by {
        println!("   {method:<24} {n:>4} requests {bytes:>8} bytes");
    }
}

#[test]
#[ignore = "reads a real Slack workspace; set RADAR_SLACK_LIVE_CONFIG"]
fn live_cold_start_refresh_and_restart() {
    let Ok(path) = std::env::var("RADAR_SLACK_LIVE_CONFIG") else {
        panic!("RADAR_SLACK_LIVE_CONFIG names a toml file with a [slack] table");
    };
    let text = std::fs::read_to_string(path).unwrap();
    let table: toml::Value = toml::from_str(&text).unwrap();
    let slack = table.get("slack").cloned().expect("a [slack] table");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let make = |cache: Option<serde_json::Value>| {
        let mut p = SlackPlugin::new();
        p.configure(Some(&slack)).unwrap();
        let token = p.cfg.resolved_token().unwrap();
        let counting = Arc::new(Counting {
            http: Http::new(&p.cfg.resolved_api().unwrap(), &token).unwrap(),
            seen: Mutex::new(Vec::new()),
        });
        p.via = crate::Via(Some(counting.clone()));
        if let Some(cache) = cache {
            p.restore(cache);
        }
        (p, counting)
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (mut p, counting) = make(None);
    let refresh = p.cfg.refresh_secs as i64;
    let started = Instant::now();
    rt.block_on(async {
        let fx = p.tick(now);
        drive(&mut p, fx).await;
    });
    report("cold start (no cache)", started.elapsed(), &counting);
    let started = Instant::now();
    rt.block_on(async {
        let fx = p.tick(now + refresh);
        drive(&mut p, fx).await;
    });
    report("refresh (finishes the threads the first read left)", started.elapsed(), &counting);
    let started = Instant::now();
    rt.block_on(async {
        let fx = p.tick(now + 2 * refresh);
        drive(&mut p, fx).await;
    });
    report("refresh again (steady)", started.elapsed(), &counting);
    let saved = p.snapshot().expect("a snapshot");
    let (mut q, counting) = make(Some(saved));
    let started = Instant::now();
    rt.block_on(async {
        let fx = q.tick(now + 3 * refresh);
        drive(&mut q, fx).await;
    });
    report("restart from the saved cache", started.elapsed(), &counting);
}
