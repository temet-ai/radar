//! A Copilot that acts out a scripted scenario, for radar-sessions' contract kit.
//!
//! Every event it emits is a copy of an event type recorded from the real CLI (the
//! fixture), with only ids and the fields a scenario controls replaced, so the adapter
//! maps real shapes. Nothing here sleeps or reads a clock: it emits synchronously, and
//! the one task per permission request waits for the adapter's answer exactly as the
//! SDK's handler does.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use radar_sessions::{BoxFuture, RadarSessionId, ModelInfo, PermissionDecision, ProviderSessionInfo};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::backend::{Backend, BackendStatus, Sink, StatusError};
use crate::map::RawEvent;
use crate::replay::Recording;

/// What the scripted provider does on a send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plot {
    Text,
    /// Asks to run `ls`; approval runs it, denial refuses it.
    Tool,
    /// Says something, then works until stopped or released.
    LongTurn,
    /// Says "Once" (twice delivered), asks to run `ls` (twice delivered), then says
    /// "Twice delivered".
    Redelivery,
    /// Asks "Which colour?" (red or blue, free text allowed) and waits; the answer is
    /// said back and the turn ends.
    Question,
}

enum Step {
    Emit(Vec<RawEvent>),
    /// Persisted events delivered to the client twice.
    EmitTwice(Vec<RawEvent>),
    Ask {
        request: RawEvent,
        twice: bool,
        approved: Vec<RawEvent>,
        denied: Vec<RawEvent>,
    },
    Hang {
        on_stop: Vec<RawEvent>,
        on_release: Vec<RawEvent>,
    },
    /// `ask_user`: the event, then the SDK's user-input handler waits for the answer.
    Question { request: RawEvent },
}

#[derive(Default)]
struct Session {
    /// Persisted events, in the order the provider made them: what a resume reads.
    history: Vec<RawEvent>,
    plots: VecDeque<Plot>,
    sink: Option<Arc<dyn Sink>>,
    hanging: Option<(Vec<RawEvent>, Vec<RawEvent>)>,
}

struct State {
    sessions: HashMap<RadarSessionId, Session>,
    connected: bool,
    tasks: Vec<JoinHandle<()>>,
}

pub struct Sim {
    rec: Recording,
    state: Mutex<State>,
    ids: AtomicU64,
    /// Bumped on every emitted event; `settled` waits for it to stop moving.
    progress: AtomicU64,
}

impl Sim {
    pub fn new(rec: Recording) -> Arc<Self> {
        Arc::new(Self {
            rec,
            state: Mutex::new(State {
                sessions: HashMap::new(),
                connected: true,
                tasks: Vec::new(),
            }),
            ids: AtomicU64::new(0),
            progress: AtomicU64::new(0),
        })
    }

    /// Queue what the next send(s) of every new session do.
    pub fn plan(&self, id: &RadarSessionId, plots: &[Plot]) {
        let mut st = self.state.lock().unwrap();
        st.sessions.entry(id.clone()).or_default().plots = plots.iter().copied().collect();
    }

    /// A radar restart: the connection and everything in flight go; history stays.
    pub fn restart(&self) {
        let mut st = self.state.lock().unwrap();
        for t in st.tasks.drain(..) {
            t.abort();
        }
        for s in st.sessions.values_mut() {
            s.sink = None;
            s.hanging = None;
        }
        st.connected = true;
    }

    /// Returns when the provider has delivered everything it will before the next action.
    pub async fn settled(&self) {
        let mut quiet = 0;
        let mut last = self.progress.load(Ordering::SeqCst);
        while quiet < 5 {
            tokio::task::yield_now().await;
            let now = self.progress.load(Ordering::SeqCst);
            if now == last {
                quiet += 1;
            } else {
                quiet = 0;
                last = now;
            }
        }
    }

    pub fn disconnect(&self) {
        let sinks: Vec<Arc<dyn Sink>> = {
            let mut st = self.state.lock().unwrap();
            st.connected = false;
            st.sessions
                .values()
                .filter_map(|s| s.sink.clone())
                .collect()
        };
        for s in sinks {
            s.lost("connection closed".into());
        }
    }

    pub fn reconnect(&self) {
        let sinks: Vec<Arc<dyn Sink>> = {
            let mut st = self.state.lock().unwrap();
            st.connected = true;
            st.sessions
                .values()
                .filter_map(|s| s.sink.clone())
                .collect()
        };
        for s in sinks {
            s.restored();
        }
    }

    /// A long turn finishes by itself (while connected its events arrive; while away they
    /// only reach the history).
    pub fn release(&self) {
        let events = {
            let mut st = self.state.lock().unwrap();
            st.sessions
                .iter_mut()
                .find_map(|(id, s)| s.hanging.take().map(|(_, rel)| (id.clone(), rel)))
        };
        if let Some((id, events)) = events {
            for e in events {
                self.emit(&id, e);
            }
        }
    }

    fn next_id(&self) -> String {
        format!(
            "00000000-0000-4000-8000-{:012}",
            self.ids.fetch_add(1, Ordering::SeqCst) + 1
        )
    }

    /// A recorded event of this type, with a new id and `patch` merged into its data.
    fn ev(&self, ty: &str, ephemeral: bool, patch: Value) -> RawEvent {
        let mut base = self
            .rec_event(ty)
            .unwrap_or_else(|| panic!("the recording holds no `{ty}` event; record again"));
        base.id = self.next_id();
        base.ephemeral = ephemeral;
        base.parent_id = None;
        if let (Some(d), Some(p)) = (base.data.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                d.insert(k.clone(), v.clone());
            }
        }
        base
    }

    fn rec_event(&self, ty: &str) -> Option<RawEvent> {
        ["turn1", "turn2", "turn2-after-abort", "turn3", "history"]
            .iter()
            .flat_map(|p| self.rec.phase(p).iter())
            .find(|e| e.event_type == ty)
            .cloned()
    }

    /// An event the recording does not hold, in the shape of the SDK's generated
    /// `session_events.rs` (`UserInputRequestedData`, `UserInputCompletedData`).
    fn unrecorded(&self, ty: &str, data: Value) -> RawEvent {
        RawEvent {
            id: self.next_id(),
            timestamp: "2026-10-14T00:00:00.000Z".into(),
            parent_id: None,
            ephemeral: false,
            agent_id: None,
            event_type: ty.into(),
            data,
        }
    }

    fn shell_request(&self, request_id: &str, tool_call: &str) -> RawEvent {
        let shell = ["turn1", "history"]
            .iter()
            .flat_map(|p| self.rec.phase(p).iter())
            .find(|e| {
                e.event_type == "permission.requested"
                    && e.data
                        .pointer("/permissionRequest/kind")
                        .and_then(Value::as_str)
                        == Some("shell")
            })
            .cloned()
            .expect("the recording holds a shell permission request");
        let mut e = shell;
        e.id = self.next_id();
        e.parent_id = None;
        e.data["requestId"] = json!(request_id);
        e.data["toolCallId"] = json!(tool_call);
        e.data["permissionRequest"]["toolCallId"] = json!(tool_call);
        e.data["permissionRequest"]["fullCommandText"] = json!("ls");
        e.data["permissionRequest"]["commandSegments"] =
            json!([{"fullCommandText": "ls", "identifier": "ls"}]);
        // The CLI offers a session approval for commands it can name (the recording's `ls` was
        // not offered one); the simulated request is, so approve for session can be checked.
        e.data["permissionRequest"]["canOfferSessionApproval"] = json!(true);
        e
    }

    fn say(&self, message_id: &str, text: &str, deltas: bool) -> Vec<RawEvent> {
        // A message id is unique per message, as Copilot's are.
        let message_id = &format!("{message_id}-{}", self.next_id());
        let mut v = vec![self.ev("assistant.turn_start", false, json!({}))];
        if deltas {
            let half = text.len() / 2;
            for part in [&text[..half], &text[half..]] {
                v.push(self.ev(
                    "assistant.message_delta",
                    true,
                    json!({"messageId": message_id, "deltaContent": part}),
                ));
            }
        }
        v.push(self.ev(
            "assistant.message",
            false,
            json!({"messageId": message_id, "content": text, "toolRequests": []}),
        ));
        v
    }

    fn end_turn(&self) -> Vec<RawEvent> {
        vec![
            self.ev("assistant.turn_end", false, json!({})),
            self.ev("session.idle", true, json!({})),
        ]
    }

    fn script(&self, plot: Plot) -> Vec<Step> {
        let tool = |call: &str| {
            self.ev("tool.execution_start", false, json!({"toolCallId": call, "toolName": "bash", "arguments": {"command": "ls", "description": "list"}}))
        };
        match plot {
            Plot::Text => vec![Step::Emit(
                [self.say("m1", "Hello there", true), self.end_turn()].concat(),
            )],
            Plot::Tool => {
                let req_id = format!("req-{}", self.next_id());
                vec![
                    Step::Emit(vec![self.ev("assistant.turn_start", false, json!({})), tool("call-1")]),
                    Step::Ask {
                        request: self.shell_request(&req_id, "call-1"),
                        twice: false,
                        approved: vec![
                            self.ev("permission.completed", false, json!({"requestId": req_id, "result": {"kind": "approved"}})),
                            self.ev("tool.execution_complete", false, json!({"toolCallId": "call-1", "success": true, "result": {"content": "a.txt\nb.md"}})),
                        ],
                        denied: vec![
                            self.ev("permission.completed", false, json!({"requestId": req_id, "result": {"kind": "denied-interactively-by-user"}})),
                            self.ev("tool.execution_complete", false, json!({"toolCallId": "call-1", "success": false, "error": {"code": "denied", "message": "The user rejected this tool call."}})),
                        ],
                    },
                    Step::Emit([self.say("m2", "That is the listing", true), self.end_turn()].concat()),
                ]
            }
            Plot::LongTurn => vec![
                Step::Emit(self.say("m1", "Working on it", true)),
                Step::Hang {
                    on_stop: vec![
                        self.ev("abort", false, json!({"reason": "user_initiated"})),
                        self.ev("assistant.turn_end", false, json!({})),
                        self.ev("session.idle", true, json!({})),
                    ],
                    on_release: [self.say("m2", "Finished", false), self.end_turn()].concat(),
                },
            ],
            Plot::Question => {
                let req_id = format!("ask-{}", self.next_id());
                vec![
                    Step::Emit(vec![self.ev("assistant.turn_start", false, json!({}))]),
                    Step::Question {
                        request: self.unrecorded(
                            "user_input.requested",
                            json!({"requestId": req_id, "question": "Which colour?", "choices": ["red", "blue"], "allowFreeform": true, "toolCallId": "call-ask"}),
                        ),
                    },
                ]
            }
            Plot::Redelivery => {
                let req_id = format!("req-{}", self.next_id());
                vec![
                    Step::EmitTwice(self.say("m1", "Once", false)),
                    Step::Emit(vec![tool("call-1")]),
                    Step::Ask {
                        request: self.shell_request(&req_id, "call-1"),
                        twice: true,
                        approved: vec![
                            self.ev("permission.completed", false, json!({"requestId": req_id, "result": {"kind": "approved"}})),
                            self.ev("tool.execution_complete", false, json!({"toolCallId": "call-1", "success": true, "result": {"content": "a.txt"}})),
                        ],
                        denied: vec![],
                    },
                    Step::Emit([self.say("m2", "Twice delivered", false), self.end_turn()].concat()),
                ]
            }
        }
    }

    /// Records a persisted event and delivers it when a client is attached.
    fn emit(&self, id: &RadarSessionId, e: RawEvent) {
        let sink = {
            let mut st = self.state.lock().unwrap();
            let connected = st.connected;
            let s = st.sessions.entry(id.clone()).or_default();
            if !e.ephemeral {
                s.history.push(e.clone());
            }
            if connected {
                s.sink.clone()
            } else {
                None
            }
        };
        if let Some(sink) = sink {
            sink.event(e);
        }
        self.progress.fetch_add(1, Ordering::SeqCst);
    }

    fn deliver_again(&self, id: &RadarSessionId, e: RawEvent) {
        let sink = self
            .state
            .lock()
            .unwrap()
            .sessions
            .get(id)
            .and_then(|s| s.sink.clone());
        if let Some(sink) = sink {
            sink.event(e);
        }
        self.progress.fetch_add(1, Ordering::SeqCst);
    }

    /// Runs steps until one waits.
    fn run(self: &Arc<Self>, id: RadarSessionId, mut steps: VecDeque<Step>) {
        while let Some(step) = steps.pop_front() {
            match step {
                Step::Emit(events) => events.into_iter().for_each(|e| self.emit(&id, e)),
                Step::EmitTwice(events) => {
                    for e in events {
                        self.emit(&id, e.clone());
                        if !e.ephemeral {
                            self.deliver_again(&id, e);
                        }
                    }
                }
                Step::Hang {
                    on_stop,
                    on_release,
                } => {
                    self.state
                        .lock()
                        .unwrap()
                        .sessions
                        .entry(id.clone())
                        .or_default()
                        .hanging = Some((on_stop, on_release));
                    return;
                }
                Step::Question { request } => {
                    let d = request.data.clone();
                    let sink = self
                        .state
                        .lock()
                        .unwrap()
                        .sessions
                        .get(&id)
                        .and_then(|s| s.sink.clone());
                    self.emit(&id, request);
                    let Some(sink) = sink else { return };
                    let this = self.clone();
                    let handle = tokio::spawn(async move {
                        // Exactly what the SDK's user-input handler gets: no request id.
                        let text = d["question"].as_str().unwrap_or_default().to_string();
                        let choices = vec!["red".to_string(), "blue".to_string()];
                        let decision = sink.question(text, choices, true).await;
                        let (answer, own) = match decision {
                            radar_sessions::QuestionDecision::Answer { replies } => {
                                let r = replies.into_iter().next();
                                (r.as_ref().map(|r| r.values.join(", ")), r.is_some_and(|r| r.own_words))
                            }
                            radar_sessions::QuestionDecision::Decline { .. } => (None, false),
                        };
                        let mut done = vec![this.unrecorded(
                            "user_input.completed",
                            json!({"requestId": d["requestId"], "answer": answer, "wasFreeform": own}),
                        )];
                        done.extend(this.say("m2", &format!("You chose {}", answer.unwrap_or_default()), false));
                        done.extend(this.end_turn());
                        let mut rest = steps;
                        rest.push_front(Step::Emit(done));
                        this.run(id, rest);
                    });
                    self.state.lock().unwrap().tasks.push(handle);
                    return;
                }
                Step::Ask {
                    request,
                    twice,
                    approved,
                    denied,
                } => {
                    let request_id = request
                        .data
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let data = request.data.clone();
                    let sink = self
                        .state
                        .lock()
                        .unwrap()
                        .sessions
                        .get(&id)
                        .and_then(|s| s.sink.clone());
                    self.emit(&id, request.clone());
                    if twice {
                        self.deliver_again(&id, request);
                    }
                    let Some(sink) = sink else { return };
                    let this = self.clone();
                    let handle = tokio::spawn(async move {
                        // Exactly what the SDK's handler does: wait for the operator.
                        let decision = sink.permission(request_id, data).await;
                        let events = match decision {
                            PermissionDecision::ApproveOnce | PermissionDecision::ApproveForSession => approved,
                            PermissionDecision::Deny { .. } => denied,
                        };
                        let mut rest = steps;
                        rest.push_front(Step::Emit(events));
                        this.run(id, rest);
                    });
                    self.state.lock().unwrap().tasks.push(handle);
                    return;
                }
            }
        }
    }

    fn send(self: &Arc<Self>, id: &RadarSessionId, text: &str) -> anyhow::Result<()> {
        let plot = {
            let mut st = self.state.lock().unwrap();
            if !st.connected {
                anyhow::bail!("not connected");
            }
            let s = st
                .sessions
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("session {id} is not open"))?;
            s.plots
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted turn left"))?
        };
        self.emit(id, self.ev("user.message", false, json!({"content": text})));
        let steps = self.script(plot).into();
        self.run(id.clone(), steps);
        Ok(())
    }

    fn stop(&self, id: &RadarSessionId) -> anyhow::Result<()> {
        let hanging = self
            .state
            .lock()
            .unwrap()
            .sessions
            .get_mut(id)
            .and_then(|s| s.hanging.take());
        if let Some((on_stop, _)) = hanging {
            for e in on_stop {
                self.emit(id, e);
            }
        }
        Ok(())
    }
}

/// [`Backend`] over a shared [`Sim`].
pub struct SimBackend(pub Arc<Sim>);

impl Backend for SimBackend {
    fn status(&self) -> BoxFuture<'_, Result<BackendStatus, StatusError>> {
        Box::pin(async {
            Ok(BackendStatus {
                version: "1.0.95".into(),
                protocol: 3,
                authenticated: true,
                login: Some("user".into()),
                host: Some("https://github.com".into()),
                message: None,
            })
        })
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
        _setup: &'a radar_sessions::adapter::SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.0
                .state
                .lock()
                .unwrap()
                .sessions
                .entry(id.clone())
                .or_default()
                .sink = Some(sink);
            Ok(())
        })
    }

    fn resume<'a>(
        &'a self,
        id: &'a RadarSessionId,
        _cwd: &'a Path,
        _setup: &'a radar_sessions::adapter::SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let history = {
                let mut st = self.0.state.lock().unwrap();
                st.connected = true;
                let s = st
                    .sessions
                    .get_mut(id)
                    .ok_or_else(|| anyhow::anyhow!("no such session {id}"))?;
                s.sink = Some(sink.clone());
                s.history.clone()
            };
            // Persisted history first, as `get_events` returns it.
            for e in history {
                sink.event(e);
            }
            self.0.progress.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn send<'a>(&'a self, id: &'a RadarSessionId, text: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { self.0.send(id, text) })
    }

    fn abort<'a>(&'a self, id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { self.0.stop(id) })
    }

    fn delete<'a>(&'a self, provider_session_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.0
                .state
                .lock()
                .unwrap()
                .sessions
                .remove(&RadarSessionId::new(provider_session_id));
            Ok(())
        })
    }

    fn list(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }
}
