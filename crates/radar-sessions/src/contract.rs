//! The contract tests every adapter runs.
//!
//! An adapter crate implements [`Harness`] (how to get its adapter into a
//! known provider behaviour, from a scripted transport or a recorded stream)
//! and calls [`run_all`] from a `#[tokio::test]`. The kit drives the adapter
//! through the public [`Adapter`] trait only, and feeds everything it
//! delivers through radar's [`Session`] ingest, so what it asserts is what the
//! dock would show.
//!
//! No sleeps, no wall clock: the harness's [`Harness::settle`] returns when
//! the provider has delivered everything it will before the next action, and
//! the kit then reads what has arrived.
//!
//! Checks that need a capability the adapter lacks are skipped; the kit says
//! which in the returned [`Report`]. A control that is absent must be
//! declared in [`Capabilities::unsupported`], never faked.

use std::path::PathBuf;

use crate::adapter::*;
use crate::event::*;
use crate::store::{
    Ingested, Item, PermissionState, Session, SessionMeta, SessionState, ToolState,
};

/// What the provider does on the first `send` after `create` (and, for
/// [`Scenario::TwoTurns`], on the second).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// Streams a short answer (at least two deltas), saves it, ends the turn.
    Text,
    /// Two short answers, one per turn.
    TwoTurns,
    /// Asks permission to run one shell command and waits. Approved: the tool
    /// completes ok and the turn then says something and ends. Denied: the
    /// tool completes not ok and the turn ends.
    Tool,
    /// Says something, then keeps working until stopped or released; on
    /// release it finishes and ends the turn.
    LongTurn,
    /// Delivers some persisted events (and a permission request) twice.
    Redelivery,
    /// Delivers one persisted event after the one that follows it.
    Reordered,
    /// Asks the operator one question offering at least two choices and
    /// waits. Answered: the provider reports the question settled, says
    /// something and ends the turn.
    Question,
}

/// How the contract kit gets at an adapter and its provider.
pub trait Harness: Send {
    /// A fresh adapter whose provider acts out `scenario`.
    fn adapter(&mut self, scenario: Scenario) -> BoxFuture<'_, Box<dyn Adapter>>;
    /// A new adapter over the same provider state (a radar restart: sessions and
    /// persisted history survive, the connection and ephemeral events do not).
    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>>;
    /// Returns when the provider has delivered everything it will until the
    /// next action.
    fn settle(&mut self) -> BoxFuture<'_, ()>;
    /// The connection to the provider drops. Persisted events keep happening.
    fn disconnect(&mut self) -> BoxFuture<'_, ()>;
    fn reconnect(&mut self) -> BoxFuture<'_, ()>;
    /// A [`Scenario::LongTurn`] finishes by itself.
    fn release(&mut self) -> BoxFuture<'_, ()>;
    /// Scenarios this provider cannot produce, each with the reason (Codex
    /// gives notifications no id or sequence, so it cannot deliver out of
    /// order). They show in [`Report::skipped`]; nothing else is skipped.
    fn skips(&self) -> Vec<(Scenario, &'static str)> {
        Vec::new()
    }
}

/// Which checks ran and which were skipped (with the reason).
#[derive(Debug, Default)]
pub struct Report {
    pub ran: Vec<&'static str>,
    pub skipped: Vec<(&'static str, String)>,
}

/// Run every check the adapter's capabilities allow.
pub async fn run_all(h: &mut dyn Harness) -> Report {
    let mut report = Report::default();
    let caps = h.adapter(Scenario::Text).await.capabilities();
    declares_what_it_lacks(&caps);
    report.ran.push("capabilities");
    if !caps.structured_events {
        // A PTY-style adapter: no events, no permissions. Only the lifecycle.
        lifecycle(h).await;
        report.ran.push("lifecycle");
        for name in ["stream", "approve", "deny", "question", "stop", "resume", "resume_waiting", "duplicates", "out_of_order", "lost_connection"] {
            report.skipped.push((name, "the adapter has no structured events".into()));
        }
        return report;
    }
    let skips = h.skips();
    let skip = |s: Scenario| skips.iter().find(|(k, _)| *k == s).map(|(_, why)| why.to_string());
    lifecycle(h).await;
    report.ran.push("lifecycle");
    stream(h).await;
    report.ran.push("stream");
    match skip(Scenario::Redelivery) {
        Some(why) => report.skipped.push(("duplicates", why)),
        None => {
            duplicates(h).await;
            report.ran.push("duplicates");
        }
    }
    match skip(Scenario::Reordered) {
        Some(why) => report.skipped.push(("out_of_order", why)),
        None => {
            out_of_order(h).await;
            report.ran.push("out_of_order");
        }
    }
    if caps.permissions {
        approve(h).await;
        report.ran.push("approve");
        approve_for_session(h).await;
        report.ran.push("approve_for_session");
        deny(h).await;
        report.ran.push("deny");
    } else {
        report.skipped.push(("approve", "no structured permissions".into()));
        report.skipped.push(("approve_for_session", "no structured permissions".into()));
        report.skipped.push(("deny", "no structured permissions".into()));
    }
    match (caps.questions, skip(Scenario::Question)) {
        (false, _) => report.skipped.push(("question", "the agent cannot ask the operator a question".into())),
        (true, Some(why)) => report.skipped.push(("question", why)),
        (true, None) => {
            question(h).await;
            report.ran.push("question");
        }
    }
    if caps.stop {
        stop(h).await;
        report.ran.push("stop");
    } else {
        report.skipped.push(("stop", "the adapter cannot stop a turn".into()));
    }
    if caps.resume && caps.permissions {
        resume_waiting(h).await;
        report.ran.push("resume_waiting");
    } else {
        report.skipped.push(("resume_waiting", "needs resume and structured permissions".into()));
    }
    if caps.resume {
        resume(h).await;
        report.ran.push("resume");
        lost_connection(h).await;
        report.ran.push("lost_connection");
    } else {
        report.skipped.push(("resume", "the adapter cannot resume".into()));
        report.skipped.push(("lost_connection", "needs resume".into()));
    }
    report
}

/// A control the adapter says it cannot do must be explained.
pub fn declares_what_it_lacks(caps: &Capabilities) {
    let lacks = [
        ("resume", caps.resume),
        ("stop", caps.stop),
        ("steer", caps.steer),
        ("list_sessions", caps.list_sessions),
        ("delete_session", caps.delete_session),
        ("list_models", caps.list_models),
        ("permissions", caps.permissions),
        ("questions", caps.questions),
        ("structured_events", caps.structured_events),
    ];
    for (control, has) in lacks {
        if !has {
            assert!(
                caps.unsupported.iter().any(|u| u.control == control && !u.reason.is_empty()),
                "capability `{control}` is false but not explained in `unsupported`"
            );
        }
    }
}

/// One session under test: the adapter, what it delivered, and radar's view of
/// it.
pub struct Run {
    pub adapter: Box<dyn Adapter>,
    pub updates: Updates,
    pub radar: RadarSessionId,
    pub provider_session_id: ProviderSessionId,
    pub session: Session,
    /// Every event as delivered, in delivery order.
    pub events: Vec<Event>,
    /// Every non-event update, in order.
    pub signals: Vec<Update>,
    /// What ingesting each delivered event did, in order.
    pub ingested: Vec<Ingested>,
}

impl Run {
    /// Subscribe, create a session in a scratch directory.
    pub async fn start(adapter: Box<dyn Adapter>, radar: &str) -> Run {
        let updates = adapter.subscribe().await.expect("subscribe");
        let radar = RadarSessionId::new(radar);
        let provider_session_id = adapter
            .create(CreateSession { radar_session_id: radar.clone(), cwd: scratch(), model: None, setup: Default::default() })
            .await
            .expect("create");
        let session = Session::new(SessionMeta {
            radar_session_id: radar.clone(),
            provider: adapter.id(),
            provider_session_id: provider_session_id.clone(),
            cwd: scratch().display().to_string(),
            title: "contract".into(),
            model: None,
            created_at: 1,
            cursor: None,
            agent: None,
            review: false,
        });
        Run { adapter, updates, radar, provider_session_id, session, events: Vec::new(), signals: Vec::new(), ingested: Vec::new() }
    }

    /// Read what has arrived (after `settle`) into radar's session.
    pub fn pump(&mut self) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::Event(e) => {
                    self.ingested.push(self.session.ingest(e.clone()));
                    self.events.push(e);
                }
                other => {
                    match &other {
                        Update::Disconnected { .. } => self.session.set_connected(false),
                        Update::Reconnected => self.session.set_connected(true),
                        Update::Event(_) | Update::Commands { .. } => {}
                    }
                    self.signals.push(other);
                }
            }
        }
    }

    pub async fn send(&mut self, h: &mut dyn Harness, text: &str) {
        self.adapter.send(&self.radar, SendInput { text: text.into() }).await.expect("send");
        h.settle().await;
        self.pump();
    }

    pub fn kinds(&self) -> Vec<&EventKind> {
        self.events.iter().map(|e| &e.kind).collect()
    }

    pub fn saw(&self, f: impl Fn(&EventKind) -> bool) -> bool {
        self.events.iter().any(|e| f(&e.kind))
    }

    pub fn persisted_assistant(&self) -> Vec<String> {
        self.session
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Assistant { text, saved: true, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
}

fn scratch() -> PathBuf {
    std::env::temp_dir()
}

/// Facts true of every event, whatever the scenario.
pub fn check_envelope(run: &Run, ordered: bool) {
    let provider = run.adapter.id();
    let mut last = 0;
    for e in &run.events {
        assert_eq!(e.provider, provider, "event names the wrong provider: {e:?}");
        assert_eq!(e.radar_session_id, run.radar, "event for another radar session: {e:?}");
        assert_eq!(e.provider_session_id, run.provider_session_id, "event for another provider session: {e:?}");
        assert!(e.observed_at > 0, "observed_at is missing: {e:?}");
        assert!(e.sequence > 0, "sequence starts at 1: {e:?}");
        if ordered {
            assert!(e.sequence > last, "sequence went {last} -> {}: {e:?}", e.sequence);
            last = e.sequence;
        }
        match &e.kind {
            EventKind::AssistantDelta { .. } | EventKind::ReasoningDelta { .. } | EventKind::Usage { .. } => {
                assert_eq!(e.durability, Durability::Ephemeral, "a delta is ephemeral: {e:?}");
            }
            EventKind::AssistantMessage { .. } | EventKind::UserMessage { .. } => {
                assert_eq!(e.durability, Durability::Persisted, "a message is persisted: {e:?}");
            }
            _ => {}
        }
        if e.durability == Durability::Persisted {
            assert!(e.provider_event_id.is_some(), "a persisted event needs a provider event id for dedupe: {e:?}");
        }
    }
}

/// The end of a turn: `TurnEnded`, or `Idle` alone from a provider whose own turn events
/// bracket each model call inside the operator's turn (Copilot), so only its idle is the end.
fn turn_over(k: &EventKind) -> bool {
    matches!(k, EventKind::TurnEnded | EventKind::Idle)
}

/// Status answers without a token and the models list is a list.
pub async fn lifecycle(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Text).await;
    let status = adapter.status().await;
    assert!(status.installed, "the harness's provider is installed: {status:?}");
    assert!(status.version.is_some(), "an installed provider reports a version");
    let dump = format!("{status:?}");
    assert!(!dump.to_lowercase().contains("token"), "status must not carry a token: {dump}");
    if adapter.capabilities().list_models {
        adapter.list_models().await.expect("list_models");
    }
    let second = adapter.subscribe().await;
    let first = adapter.subscribe().await;
    // subscribe is once per adapter: exactly one of two calls may succeed.
    assert!(first.is_ok() != second.is_ok(), "subscribe is called once per adapter");
}

/// create + send: deltas are ephemeral, the saved message replaces them, the
/// turn ends and the session is idle.
pub async fn stream(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Text).await;
    let mut run = Run::start(adapter, "contract-stream").await;
    run.send(h, "hello").await;
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::UserMessage { .. })), "the user message is echoed as a persisted event");
    assert!(run.saw(|k| matches!(k, EventKind::AssistantDelta { .. })), "text streams as deltas");
    assert!(run.saw(|k| matches!(k, EventKind::AssistantMessage { .. })), "the message is saved");
    assert!(run.saw(turn_over), "the turn ends");
    let saved = run.persisted_assistant();
    assert_eq!(saved.len(), 1, "one saved assistant message: {:?}", run.session.items);
    assert!(
        run.session.items.iter().all(|i| !matches!(i, Item::Assistant { saved: false, .. })),
        "no unsaved text is left once the message is saved: {:?}",
        run.session.items
    );
    assert_eq!(run.session.state, SessionState::Idle);
    assert!(run.session.meta.cursor.is_some(), "the cursor follows persisted events");
}

/// A permission request waits for the operator, and approving runs the tool.
pub async fn approve(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Tool).await;
    let mut run = Run::start(adapter, "contract-approve").await;
    run.send(h, "run something").await;
    let request = waiting_request(&run);
    assert!(
        !run.saw(|k| matches!(k, EventKind::ToolCompleted { .. } | EventKind::PermissionResolved { .. })),
        "nothing auto-approves: the tool must not finish before the operator answers"
    );
    assert_eq!(run.session.state, SessionState::WaitingApproval);
    let answer = PermissionAnswer { request_id: request.request_id.clone(), decision: PermissionDecision::ApproveOnce };
    run.adapter.answer_permission(&run.radar, answer.clone()).await.expect("answer");
    h.settle().await;
    run.pump();
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::PermissionResolved { approved: true, .. })), "approval is reported");
    assert!(
        run.session.items.iter().any(|i| matches!(i, Item::Tool { state: ToolState::Ok, .. })),
        "the approved tool completes ok: {:?}",
        run.session.items
    );
    assert!(run.session.pending().is_empty());
    // Answering again (a request delivered twice) is a no-op, not an error and not a second run.
    let before = run.events.len();
    run.adapter.answer_permission(&run.radar, answer).await.expect("a repeated answer is idempotent");
    h.settle().await;
    run.pump();
    assert_eq!(run.events.len(), before, "a repeated answer changes nothing");
    assert_eq!(run.session.state, SessionState::Idle);
}

/// "Approve for session" is offered on the contract's request and approves
/// it: the tool runs, and answering again is a no-op. What the provider
/// remembers is its own (each adapter's tests check the wire); radar remembers
/// the rest.
pub async fn approve_for_session(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Tool).await;
    let mut run = Run::start(adapter, "contract-approve-session").await;
    run.send(h, "run something").await;
    let request = waiting_request(&run);
    assert!(request.can_offer_session_approval, "the contract's shell request offers approve for session: {request:?}");
    let answer = PermissionAnswer { request_id: request.request_id.clone(), decision: PermissionDecision::ApproveForSession };
    run.adapter.answer_permission(&run.radar, answer.clone()).await.expect("answer");
    h.settle().await;
    run.pump();
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::PermissionResolved { approved: true, .. })), "the approval is reported");
    assert!(
        run.session.items.iter().any(|i| matches!(i, Item::Tool { state: ToolState::Ok, .. })),
        "the tool approved for the session completes ok: {:?}",
        run.session.items
    );
    let before = run.events.len();
    run.adapter.answer_permission(&run.radar, answer).await.expect("a repeated answer is idempotent");
    h.settle().await;
    run.pump();
    assert_eq!(run.events.len(), before, "a repeated answer changes nothing");
}

/// Denying refuses the tool; the model hears the refusal.
pub async fn deny(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Tool).await;
    let mut run = Run::start(adapter, "contract-deny").await;
    run.send(h, "run something").await;
    let request = waiting_request(&run);
    let answer = PermissionAnswer {
        request_id: request.request_id.clone(),
        decision: PermissionDecision::Deny { reason: Some("not now".into()) },
    };
    run.adapter.answer_permission(&run.radar, answer).await.expect("answer");
    h.settle().await;
    run.pump();
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::PermissionResolved { approved: false, .. })), "denial is reported");
    assert!(
        run.session.items.iter().any(|i| matches!(i, Item::Tool { state: ToolState::Failed, .. })),
        "the denied tool does not run: {:?}",
        run.session.items
    );
    assert!(run.session.items.iter().any(|i| matches!(i, Item::Permission { state: PermissionState::Denied, .. })));
}

fn waiting_request(run: &Run) -> PermissionRequest {
    let pending = run.session.pending();
    assert_eq!(pending.len(), 1, "exactly one request waits: {:?}", run.session.items);
    let request = pending[0].clone();
    match (&request.kind, &request.target) {
        (PermissionKind::Shell, PermissionTarget::Command { command, .. }) => {
            assert!(!command.is_empty(), "the exact command is shown")
        }
        other => panic!("the contract scenario asks to run a shell command, got {other:?}"),
    }
    assert!(!request.request_id.is_empty(), "the provider request id is kept");
    request
}

/// A question waits for the operator with its choices; nothing answers it
/// but the operator, and the answer reaches the agent once.
pub async fn question(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Question).await;
    let mut run = Run::start(adapter, "contract-question").await;
    run.send(h, "ask me something").await;
    let pending = run.session.pending_questions();
    assert_eq!(pending.len(), 1, "exactly one question waits: {:?}", run.session.items);
    let request = pending[0].clone();
    assert!(!request.request_id.is_empty(), "the provider request id is kept");
    let q = request.questions.first().expect("the request asks something");
    assert!(!q.text.is_empty(), "the question is shown as asked");
    assert!(q.choices.len() >= 2, "the scenario offers two choices: {q:?}");
    assert!(
        !run.saw(|k| matches!(k, EventKind::QuestionResolved { .. })),
        "nothing answers for the operator: the question must not settle before the operator answers"
    );
    assert_eq!(run.session.state, SessionState::WaitingApproval);
    let replies = request
        .questions
        .iter()
        .map(|q| QuestionReply { question_id: q.id.clone(), values: vec![q.choices[0].label.clone()], own_words: false })
        .collect();
    let answer = QuestionAnswer { request_id: request.request_id.clone(), decision: QuestionDecision::Answer { replies } };
    run.adapter.answer_question(&run.radar, answer.clone()).await.expect("answer");
    h.settle().await;
    run.pump();
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::QuestionResolved { replies: Some(_), .. })), "the answer is reported");
    assert!(run.session.pending_questions().is_empty(), "{:?}", run.session.items);
    assert!(run.saw(turn_over), "the turn goes on and ends");
    // Answering again (a request delivered twice) is a no-op, not an error and not a second answer.
    let before = run.events.len();
    run.adapter.answer_question(&run.radar, answer).await.expect("a repeated answer is idempotent");
    h.settle().await;
    run.pump();
    assert_eq!(run.events.len(), before, "a repeated answer changes nothing");
    assert_eq!(run.session.state, SessionState::Idle);
}

/// Stop aborts the turn; the session stays usable.
pub async fn stop(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::LongTurn).await;
    let mut run = Run::start(adapter, "contract-stop").await;
    run.send(h, "keep going").await;
    assert_eq!(run.session.state, SessionState::Running, "the turn is still running: {:?}", run.session.items);
    run.adapter.stop(&run.radar).await.expect("stop");
    h.settle().await;
    run.pump();
    check_envelope(&run, true);
    assert!(run.saw(|k| matches!(k, EventKind::Aborted)), "a persisted abort is reported");
    assert_eq!(run.session.state, SessionState::Stopped);
    assert!(!run.saw(|k| matches!(k, EventKind::AssistantMessage { text, .. } if text == "Finished")), "a stopped turn does not finish");
}

/// After a restart, the provider's persisted history rebuilds the session, no
/// ephemeral event is replayed, a known cursor replays nothing new, and the
/// session works afterwards.
pub async fn resume(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::TwoTurns).await;
    let mut run = Run::start(adapter, "contract-resume").await;
    run.send(h, "first").await;
    let before = run.session.items.clone();
    let cursor = run.session.meta.cursor.clone();
    let provider_session_id = run.provider_session_id.clone();
    let radar = run.radar.clone();
    drop(run);

    // radar restarts: new adapter, empty timeline, no cursor.
    let adapter = h.restart().await;
    let mut run = Run::start_resumed(adapter, radar.clone(), provider_session_id.clone()).await;
    run.adapter
        .resume(ResumeSession { radar_session_id: radar.clone(), provider_session_id: provider_session_id.clone(), cwd: scratch(), cursor: None, setup: Default::default() })
        .await
        .expect("resume");
    h.settle().await;
    run.pump();
    check_envelope(&run, false);
    assert!(
        run.events.iter().all(|e| e.durability == Durability::Persisted),
        "history replay carries persisted events only: {:?}",
        run.events.iter().filter(|e| e.durability != Durability::Persisted).collect::<Vec<_>>()
    );
    assert_eq!(run.session.items, before, "the resumed history equals what was seen live");

    // A reconnect with the cursor: nothing new, nothing doubled.
    let known = run.events.len();
    run.adapter
        .resume(ResumeSession { radar_session_id: radar.clone(), provider_session_id, cwd: scratch(), cursor, setup: Default::default() })
        .await
        .expect("resume with cursor");
    h.settle().await;
    run.pump();
    assert!(
        run.ingested[known..].iter().all(|i| *i != Ingested::Applied),
        "resuming from the cursor must not change the session: {:?}",
        &run.events[known..]
    );
    assert_eq!(run.session.items, before);

    // The resumed session works.
    run.send(h, "second").await;
    assert_eq!(run.persisted_assistant().len(), 2, "the resumed session answers: {:?}", run.session.items);
    assert_eq!(run.session.state, SessionState::Idle);
}

/// radar closes while a request waits, and starts again: the provider's
/// process that asked is gone. The resumed history replays persisted events
/// only; a request it holds (Copilot keeps one, Claude Code and Codex may
/// not) comes back as it was asked, and lapses when the history ends, as the
/// part does it ([`Session::lapse`]): nothing waits, nothing needs the
/// operator, and the turn that waited is over.
pub async fn resume_waiting(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Tool).await;
    let mut run = Run::start(adapter, "contract-resume-waiting").await;
    run.send(h, "run something").await;
    let asked = waiting_request(&run);
    let (radar, provider_session_id) = (run.radar.clone(), run.provider_session_id.clone());
    drop(run);

    let adapter = h.restart().await;
    let mut run = Run::start_resumed(adapter, radar.clone(), provider_session_id.clone()).await;
    run.adapter
        .resume(ResumeSession { radar_session_id: radar, provider_session_id, cwd: scratch(), cursor: None, setup: Default::default() })
        .await
        .expect("resume after the restart");
    h.settle().await;
    run.pump();
    check_envelope(&run, false);
    assert!(
        run.events.iter().all(|e| e.durability == Durability::Persisted),
        "history replay carries persisted events only: {:?}",
        run.events.iter().filter(|e| e.durability != Durability::Persisted).collect::<Vec<_>>()
    );
    let replayed = run.saw(|k| matches!(k, EventKind::PermissionRequested { request } if request.request_id == asked.request_id));
    let lapsed = run.session.lapse();
    assert!(run.session.pending().is_empty() && run.session.pending_questions().is_empty(), "nothing waits after the history: {:?}", run.session.items);
    assert!(!run.session.busy(), "the turn that waited is over: {:?} {:?}", run.session.state, run.session.items);
    if replayed {
        assert_eq!(lapsed.requests, 1, "the request the history holds lapses: {:?}", run.session.items);
        assert!(
            run.session.items.iter().any(|i| matches!(i, Item::Permission { request, state: PermissionState::Lapsed } if request.request_id == asked.request_id)),
            "{:?}",
            run.session.items
        );
    }
}

/// Events delivered twice change nothing the second time; a permission
/// request delivered twice is still one request.
pub async fn duplicates(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Redelivery).await;
    let mut run = Run::start(adapter, "contract-dup").await;
    run.send(h, "go").await;
    assert!(
        run.ingested.contains(&Ingested::Duplicate),
        "the scenario redelivers events; the store must see duplicates"
    );
    assert_eq!(run.session.pending().len(), 1, "a request delivered twice is one request: {:?}", run.session.items);
    let request = waiting_request(&run);
    run.adapter
        .answer_permission(&run.radar, PermissionAnswer { request_id: request.request_id, decision: PermissionDecision::ApproveOnce })
        .await
        .expect("answer");
    h.settle().await;
    run.pump();
    assert_eq!(run.persisted_assistant(), vec!["Once".to_string(), "Twice delivered".to_string()]);
    assert!(
        run.session.items.iter().filter(|i| matches!(i, Item::Permission { .. })).count() == 1,
        "one permission item: {:?}",
        run.session.items
    );
}

/// Persisted events delivered out of order end up in order.
pub async fn out_of_order(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::Reordered).await;
    let mut run = Run::start(adapter, "contract-order").await;
    run.send(h, "go").await;
    check_envelope(&run, false);
    let delivered: Vec<u64> = run.events.iter().filter(|e| e.durability == Durability::Persisted).map(|e| e.sequence).collect();
    assert!(delivered.windows(2).any(|w| w[0] > w[1]), "the scenario delivers out of order: {delivered:?}");
    assert_eq!(
        run.persisted_assistant(),
        vec!["Late first".to_string(), "Then second".to_string()],
        "items follow the provider's sequence: {:?}",
        run.session.items
    );
}

/// The connection drops mid-turn and returns; resuming from the cursor
/// delivers what was missed, once.
pub async fn lost_connection(h: &mut dyn Harness) {
    let adapter = h.adapter(Scenario::LongTurn).await;
    let mut run = Run::start(adapter, "contract-lost").await;
    run.send(h, "keep going").await;
    assert_eq!(run.session.state, SessionState::Running);
    h.disconnect().await;
    h.settle().await;
    run.pump();
    assert!(run.signals.iter().any(|s| matches!(s, Update::Disconnected { .. })), "the loss is reported");
    assert_eq!(run.session.state, SessionState::Disconnected, "a lost connection is its own state");
    assert!(
        run.adapter.send(&run.radar, SendInput { text: "while away".into() }).await.is_err(),
        "sending without a connection fails loudly"
    );
    // The turn finishes while we are away.
    h.release().await;
    h.settle().await;
    run.pump();
    assert!(!run.saw(|k| matches!(k, EventKind::AssistantMessage { text, .. } if text == "Finished")), "nothing arrives while disconnected");

    h.reconnect().await;
    h.settle().await;
    run.pump();
    assert!(run.signals.iter().any(|s| matches!(s, Update::Reconnected)), "the return is reported");
    let cursor = run.session.meta.cursor.clone();
    run.adapter
        .resume(ResumeSession { radar_session_id: run.radar.clone(), provider_session_id: run.provider_session_id.clone(), cwd: scratch(), cursor, setup: Default::default() })
        .await
        .expect("resume after reconnect");
    h.settle().await;
    run.pump();
    check_envelope(&run, false);
    assert_eq!(
        run.persisted_assistant(),
        vec!["Working on it".to_string(), "Finished".to_string()],
        "the missed message arrives once: {:?}",
        run.session.items
    );
    assert_eq!(run.session.state, SessionState::Idle);
}

impl Run {
    /// A run over an existing provider session (after a restart): the
    /// session starts empty with no cursor.
    pub async fn start_resumed(adapter: Box<dyn Adapter>, radar: RadarSessionId, provider_session_id: ProviderSessionId) -> Run {
        let updates = adapter.subscribe().await.expect("subscribe");
        let session = Session::new(SessionMeta {
            radar_session_id: radar.clone(),
            provider: adapter.id(),
            provider_session_id: provider_session_id.clone(),
            cwd: scratch().display().to_string(),
            title: "contract".into(),
            model: None,
            created_at: 1,
            cursor: None,
            agent: None,
            review: false,
        });
        Run { adapter, updates, radar, provider_session_id, session, events: Vec::new(), signals: Vec::new(), ingested: Vec::new() }
    }
}
