//! A [`Backend`] that replays a stream recorded once from the real CLI, so the adapter's
//! tests run without Copilot, a network or a clock.
//!
//! The recording is JSON lines: each line is a [`RawEvent`] plus a `phase`
//! (`turn1`, `turn2`, `turn2-after-abort`, `history`, ...). The nth `send` plays the nth
//! entry of `turns`; `abort` plays `<turn>-after-abort`; `resume` plays `history`.
//! A recorded `permission.requested` is delivered, then the replay waits for the
//! operator's answer through the sink before playing on, as the live CLI does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use radar_sessions::adapter::{Catalog, ProviderSetup, SessionSetup};
use radar_sessions::{BoxFuture, RadarSessionId, ModelInfo, ProviderSessionInfo};
use serde::Deserialize;

use crate::backend::{Backend, BackendStatus, Sink, StatusError};
use crate::map::RawEvent;

#[derive(Deserialize)]
struct Line {
    phase: String,
    #[serde(flatten)]
    event: RawEvent,
}

#[derive(Clone, Default)]
pub struct Recording {
    phases: HashMap<String, Vec<RawEvent>>,
}

impl Recording {
    pub fn from_jsonl(text: &str) -> anyhow::Result<Self> {
        let mut phases: HashMap<String, Vec<RawEvent>> = HashMap::new();
        for (n, l) in text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
        {
            let line: Line = serde_json::from_str(l)
                .map_err(|e| anyhow::anyhow!("recording line {}: {e}", n + 1))?;
            phases.entry(line.phase).or_default().push(line.event);
        }
        Ok(Self { phases })
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Self::from_jsonl(&std::fs::read_to_string(path)?)
    }

    pub fn phase(&self, name: &str) -> &[RawEvent] {
        self.phases.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[derive(Default)]
struct Live {
    sinks: HashMap<RadarSessionId, Arc<dyn Sink>>,
    sent: usize,
    deleted: Vec<String>,
    last_turn: Option<String>,
    /// What the adapter handed the backend, in order, for tests to assert.
    created: Vec<SessionSetup>,
    resumed: Vec<SessionSetup>,
    configured: Vec<ProviderSetup>,
    catalogs: Vec<(PathBuf, SessionSetup)>,
}

pub struct ReplayBackend {
    rec: Recording,
    turns: Vec<String>,
    status: Result<BackendStatus, StatusError>,
    live: Arc<Mutex<Live>>,
    catalog: Catalog,
    /// What `configure` answers (a note for the log).
    configure_note: Option<String>,
}

impl ReplayBackend {
    pub fn new(rec: Recording, turns: &[&str]) -> Self {
        Self {
            rec,
            turns: turns.iter().map(|t| t.to_string()).collect(),
            status: Ok(BackendStatus {
                version: "1.0.95".into(),
                protocol: 3,
                authenticated: true,
                login: Some("user".into()),
                host: Some("https://github.com".into()),
                message: None,
            }),
            live: Arc::default(),
            catalog: Catalog::default(),
            configure_note: None,
        }
    }

    /// The agents and skills `catalog` answers with.
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// A note `configure` returns, as the real backend does while sessions are open.
    pub fn with_configure_note(mut self, note: &str) -> Self {
        self.configure_note = Some(note.into());
        self
    }

    /// The setups of every `create`, in order.
    pub fn created(&self) -> Vec<SessionSetup> {
        self.live.lock().unwrap().created.clone()
    }

    /// The setups of every `resume`, in order.
    pub fn resumed(&self) -> Vec<SessionSetup> {
        self.live.lock().unwrap().resumed.clone()
    }

    /// Every `configure`, in order.
    pub fn configured(&self) -> Vec<ProviderSetup> {
        self.live.lock().unwrap().configured.clone()
    }

    /// Every `catalog` asked, in order.
    pub fn catalogs(&self) -> Vec<(PathBuf, SessionSetup)> {
        self.live.lock().unwrap().catalogs.clone()
    }

    pub fn with_status(mut self, status: Result<BackendStatus, StatusError>) -> Self {
        self.status = status;
        self
    }

    /// Provider session ids deleted so far.
    pub fn deleted(&self) -> Vec<String> {
        self.live.lock().unwrap().deleted.clone()
    }

    /// Simulates losing the connection to Copilot for a session.
    pub fn drop_connection(&self, id: &RadarSessionId) {
        if let Some(s) = self.live.lock().unwrap().sinks.get(id) {
            s.lost("connection closed".into());
        }
    }

    fn play(&self, events: Vec<RawEvent>, sink: Arc<dyn Sink>) {
        tokio::spawn(async move {
            for ev in events {
                let request = (ev.event_type == "permission.requested").then(|| {
                    (
                        ev.data
                            .get("requestId")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        ev.data.clone(),
                    )
                });
                sink.event(ev);
                if let Some((id, data)) = request {
                    let _ = sink.permission(id, data).await;
                }
            }
        });
    }
}

impl Backend for ReplayBackend {
    fn status(&self) -> BoxFuture<'_, Result<BackendStatus, StatusError>> {
        let s = self.status.clone();
        Box::pin(async move { s })
    }

    fn models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        Box::pin(async {
            Ok(vec![ModelInfo {
                id: "auto".into(),
                name: "Auto".into(),
            }])
        })
    }

    fn create<'a>(
        &'a self,
        id: &'a RadarSessionId,
        _cwd: &'a Path,
        _model: Option<&'a str>,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut live = self.live.lock().unwrap();
            live.created.push(setup.clone());
            live.sinks.insert(id.clone(), sink);
            Ok(())
        })
    }

    fn resume<'a>(
        &'a self,
        id: &'a RadarSessionId,
        _cwd: &'a Path,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            {
                let mut live = self.live.lock().unwrap();
                live.resumed.push(setup.clone());
                live.sinks.insert(id.clone(), sink.clone());
            }
            // History is delivered before resume returns, as the real backend does.
            for ev in self.rec.phase("history") {
                sink.event(ev.clone());
            }
            Ok(())
        })
    }

    fn send<'a>(
        &'a self,
        id: &'a RadarSessionId,
        _text: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let (turn, sink) = {
                let mut l = self.live.lock().unwrap();
                let sink = l
                    .sinks
                    .get(id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("session {id} is not open"))?;
                let turn = self
                    .turns
                    .get(l.sent)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no recorded turn left"))?;
                l.sent += 1;
                l.last_turn = Some(turn.clone());
                (turn, sink)
            };
            self.play(self.rec.phase(&turn).to_vec(), sink);
            Ok(())
        })
    }

    fn abort<'a>(&'a self, id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let (turn, sink) = {
                let l = self.live.lock().unwrap();
                let sink = l
                    .sinks
                    .get(id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("session {id} is not open"))?;
                (l.last_turn.clone().unwrap_or_default(), sink)
            };
            self.play(
                self.rec.phase(&format!("{turn}-after-abort")).to_vec(),
                sink,
            );
            Ok(())
        })
    }

    fn delete<'a>(&'a self, provider_session_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.live
                .lock()
                .unwrap()
                .deleted
                .push(provider_session_id.to_string());
            Ok(())
        })
    }

    fn list(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn configure(&self, setup: ProviderSetup) -> Option<String> {
        self.live.lock().unwrap().configured.push(setup);
        self.configure_note.clone()
    }

    fn catalog<'a>(
        &'a self,
        cwd: &'a Path,
        setup: &'a SessionSetup,
    ) -> BoxFuture<'a, anyhow::Result<Catalog>> {
        Box::pin(async move {
            self.live
                .lock()
                .unwrap()
                .catalogs
                .push((cwd.to_path_buf(), setup.clone()));
            Ok(self.catalog.clone())
        })
    }
}
