//! The adapter against replayed app-server conversations: one recorded from
//! the real `codex app-server` 0.158.0 (tests/fixtures), the rest scripted
//! for what a single live run cannot show (a deny, a repeat, a crash).

use std::sync::Arc;
use std::time::Duration;

use radar_sessions::adapter::{ProviderSetup, SessionSetup, Updates};
use radar_sessions::{
    Adapter, AuthStatus, CreateSession, Durability, Event, EventKind, RadarSessionId,
    PermissionAnswer, PermissionDecision, PermissionTarget, ProviderSessionId, ResumeSession,
    SendInput, Update,
};
use serde_json::{Value, json};

use crate::replay::{
    self, ReplaySpawner, Seg, close, load, note, notify, request, response, result, server_request,
};
use crate::{CodexAdapter, CodexConfig, SessionPolicy};

const PHASE1: &str = include_str!("../tests/fixtures/phase1.jsonl");
const PHASE2: &str = include_str!("../tests/fixtures/phase2.jsonl");
const THREAD: &str = "019a0000-0000-7000-8000-000000000001";

fn adapter(scripts: Vec<Vec<Seg>>) -> (CodexAdapter, Arc<ReplaySpawner>) {
    let spawner = ReplaySpawner::new(scripts);
    let cfg = CodexConfig {
        clock: Arc::new(|| 1234),
        ..CodexConfig::default()
    };
    (
        CodexAdapter::with(Box::new(Arc::clone(&spawner)), cfg),
        spawner,
    )
}

/// Let the fake server read what the adapter just wrote.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

fn radar() -> RadarSessionId {
    RadarSessionId::new("g1")
}

fn create_req() -> CreateSession {
    CreateSession {
        radar_session_id: radar(),
        cwd: "/work/repo".into(),
        model: None,
        setup: SessionSetup {
            project_config: true,
            ..Default::default()
        },
    }
}

/// Collect updates up to and including the first that satisfies `stop`.
async fn until(rx: &mut Updates, stop: impl Fn(&Update) -> bool) -> Vec<Update> {
    let mut seen = Vec::new();
    loop {
        let u = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an update in time")
            .expect("the stream stays open");
        let done = stop(&u);
        seen.push(u);
        if done {
            return seen;
        }
    }
}

fn events(u: &[Update]) -> Vec<&Event> {
    u.iter()
        .filter_map(|u| {
            if let Update::Event(e) = u {
                Some(e)
            } else {
                None
            }
        })
        .collect()
}

fn is_permission(u: &Update) -> bool {
    matches!(u, Update::Event(e) if matches!(e.kind, EventKind::PermissionRequested { .. }))
}

fn is_idle(u: &Update) -> bool {
    matches!(u, Update::Event(e) if e.kind == EventKind::Idle)
}

fn request_id(u: &[Update]) -> String {
    match &events(u).last().unwrap().kind {
        EventKind::PermissionRequested { request } => request.request_id.clone(),
        k => panic!("not a permission request: {k:?}"),
    }
}

fn answer(id: &str, decision: PermissionDecision) -> PermissionAnswer {
    PermissionAnswer {
        request_id: id.into(),
        decision,
    }
}

fn base() -> Vec<Seg> {
    vec![
        request("initialize", vec![result(json!({ "userAgent": "test" }))]),
        notify("initialized"),
    ]
}

/// A scripted conversation with one command approval (`available` decisions).
fn approval_script(available: Value) -> Vec<Seg> {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(
            json!({ "thread": { "id": "t1" }, "model": "gpt-test" }),
        )],
    ));
    s.push(request(
        "turn/start",
        vec![
            result(json!({ "turn": { "id": "u1" } })),
            note("turn/started", json!({ "threadId": "t1", "turn": { "id": "u1", "status": "inProgress" } })),
            server_request(
                7,
                "item/commandExecution/requestApproval",
                json!({ "threadId": "t1", "turnId": "u1", "itemId": "i1", "command": "rm -rf x", "availableDecisions": available }),
            ),
        ],
    ));
    s.push(response(
        json!(7),
        vec![
            note("serverRequest/resolved", json!({ "threadId": "t1", "requestId": 7 })),
            note("turn/completed", json!({ "threadId": "t1", "turn": { "id": "u1", "status": "completed", "items": [] } })),
        ],
    ));
    s
}

// ---- status -----------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn status_reads_version_and_who_is_signed_in() {
    let (a, _) = adapter(vec![load(PHASE1)]);
    let s = a.status().await;
    assert!(s.installed);
    assert_eq!(s.version.as_deref(), Some("0.158.0"));
    assert_eq!(
        s.auth,
        AuthStatus::SignedIn {
            as_user: "operator@example.com".into()
        }
    );
    assert_eq!(s.error, None);
    assert!(s.capabilities.permissions && s.capabilities.resume && s.capabilities.stop);
    assert!(
        !s.capabilities.unsupported.is_empty(),
        "what Codex lacks is said, not hidden"
    );
}

#[tokio::test(start_paused = true)]
async fn status_tells_a_missing_executable_from_a_signed_out_cli() {
    let a = CodexAdapter::with(
        Box::new(ReplaySpawner::not_installed()),
        CodexConfig::default(),
    );
    let s = a.status().await;
    assert!(!s.installed);
    assert!(s.error.unwrap().contains("not found"));

    let mut script = base();
    script.push(request(
        "account/read",
        vec![result(
            json!({ "account": null, "requiresOpenaiAuth": true }),
        )],
    ));
    let (a, _) = adapter(vec![script]);
    let s = a.status().await;
    assert!(s.installed);
    assert_eq!(s.auth, AuthStatus::SignedOut);
}

#[tokio::test(start_paused = true)]
async fn status_says_when_the_app_server_will_not_start() {
    let (a, _) = adapter(vec![]);
    let s = a.status().await;
    assert!(s.installed);
    assert!(s.error.unwrap().contains("cannot start"));
}

#[tokio::test(start_paused = true)]
async fn models_come_from_the_app_server_without_the_hidden_ones() {
    let mut script = base();
    script.push(request(
        "model/list",
        vec![result(json!({ "data": [
            { "id": "m1", "displayName": "Model One", "hidden": false },
            { "id": "m2", "displayName": "Secret", "hidden": true }
        ] }))],
    ));
    let (a, _) = adapter(vec![script]);
    let models = a.list_models().await.unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(
        (models[0].id.as_str(), models[0].name.as_str()),
        ("m1", "Model One")
    );
}

// ---- the recorded session ---------------------------------------------------

#[tokio::test(start_paused = true)]
async fn a_recorded_session_streams_asks_and_stops() {
    let (a, srv) = adapter(vec![load(PHASE1)]);
    let mut rx = a.subscribe().await.unwrap();
    assert!(a.subscribe().await.is_err(), "one subscriber");

    let provider = a.create(create_req()).await.unwrap();
    assert_eq!(provider, ProviderSessionId::new(THREAD));
    // An ordinary thread takes the operator's own Codex config: radar sends no policy.
    let start = &srv.sent("thread/start")[0];
    assert!(start.get("approvalPolicy").is_none() && start.get("sandbox").is_none(), "{start}");

    a.send(
        &radar(),
        SendInput {
            text: "Run `ls -a`, then create hello.txt".into(),
        },
    )
    .await
    .unwrap();
    let first = until(&mut rx, is_permission).await;
    let ev = events(&first);
    assert!(ev.iter().any(
        |e| matches!(&e.kind, EventKind::UserMessage { text } if text.contains("ls -a"))
            && e.durability == Durability::Persisted
    ));
    assert!(
        ev.iter()
            .any(|e| matches!(e.kind, EventKind::AssistantDelta { .. })
                && e.durability == Durability::Ephemeral)
    );
    assert!(ev.iter().any(|e| matches!(&e.kind, EventKind::AssistantMessage { text, .. } if text.contains("list the directory"))
        && e.durability == Durability::Persisted));
    let started = ev
        .iter()
        .find(|e| matches!(e.kind, EventKind::ToolStarted { .. }))
        .unwrap();
    assert!(
        matches!(&started.kind, EventKind::ToolStarted { name, summary, .. } if name == "shell" && summary.contains("ls -a"))
    );
    let first_id = request_id(&first);
    let EventKind::PermissionRequested { request } = &ev.last().unwrap().kind else {
        unreachable!()
    };
    assert!(
        matches!(&request.target, PermissionTarget::Command { command, .. } if command.contains("ls -a"))
    );
    // Nothing answered for the operator.
    assert!(srv.answers().is_empty());

    a.answer_permission(&radar(), answer(&first_id, PermissionDecision::ApproveOnce))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        srv.answers()[0],
        json!({ "id": 0, "result": { "decision": "accept" } })
    );
    // Answering again is not an error and answers nothing twice.
    a.answer_permission(&radar(), answer(&first_id, PermissionDecision::ApproveOnce))
        .await
        .unwrap();
    settle().await;
    assert_eq!(srv.answers().len(), 1);

    let second = until(&mut rx, is_permission).await;
    assert!(
        events(&second)
            .iter()
            .any(|e| matches!(e.kind, EventKind::PermissionResolved { approved: true, .. }))
    );
    let second_id = request_id(&second);
    a.answer_permission(&radar(), answer(&second_id, PermissionDecision::ApproveOnce))
        .await
        .unwrap();
    let rest = until(&mut rx, is_idle).await;
    assert!(events(&rest).iter().any(|e| matches!(&e.kind, EventKind::AssistantMessage { text, .. } if text.contains("created `hello.txt`"))));
    assert!(events(&rest).iter().any(|e| e.kind == EventKind::TurnEnded));

    // A second turn, stopped while it waits on an approval.
    a.send(
        &radar(),
        SendInput {
            text: "Run `sleep 20`".into(),
        },
    )
    .await
    .unwrap();
    let third = until(&mut rx, is_permission).await;
    assert!(!third.is_empty());
    a.stop(&radar()).await.unwrap();
    // Codex ends the turn, then reports the unanswered request closed.
    let mut stopped = until(&mut rx, is_idle).await;
    stopped.extend(until(&mut rx, |u| matches!(u, Update::Event(e) if matches!(e.kind, EventKind::PermissionResolved { .. }))).await);
    let kinds: Vec<_> = events(&stopped).iter().map(|e| e.kind.clone()).collect();
    assert!(kinds.contains(&EventKind::Aborted), "{kinds:?}");
    assert!(kinds.iter().any(|k| matches!(
        k,
        EventKind::PermissionResolved {
            approved: false,
            ..
        }
    )));
    let interrupt = &srv.sent("turn/interrupt")[0];
    assert_eq!(interrupt["threadId"], THREAD);

    // Everything: stamped by the injected clock, in order, ids unique per persisted event.
    assert!(
        srv.log.lock().unwrap().unexpected.is_empty(),
        "{:?}",
        srv.log.lock().unwrap().unexpected
    );
}

#[tokio::test(start_paused = true)]
async fn events_are_stamped_in_order_with_stable_ids() {
    let (a, _) = adapter(vec![load(PHASE1)]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "go".into() })
        .await
        .unwrap();
    let seen = until(&mut rx, is_permission).await;
    let ev = events(&seen);
    let seqs: Vec<u64> = ev.iter().map(|e| e.sequence).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(seqs[0], 1);
    assert!(ev.iter().all(|e| e.observed_at == 1234
        && e.provider.as_str() == "codex"
        && e.radar_session_id == radar()));
    let ids: Vec<_> = ev
        .iter()
        .filter(|e| e.durability == Durability::Persisted)
        .map(|e| e.provider_event_id.clone().unwrap())
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len(), "persisted ids are unique: {ids:?}");
    assert!(
        ev.iter()
            .filter(|e| e.durability == Durability::Ephemeral)
            .all(|e| {
                matches!(
                    e.kind,
                    EventKind::AssistantDelta { .. }
                        | EventKind::Usage { .. }
                        | EventKind::ModelChanged { .. }
                        | EventKind::PermissionRequested { .. }
                        | EventKind::ReasoningDelta { .. }
                )
            })
    );
}

// ---- permissions ------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn deny_declines_when_offered_and_cancels_when_not() {
    for (available, want) in [
        (json!(["accept", "decline", "cancel"]), "decline"),
        (json!(["accept", "cancel"]), "cancel"),
    ] {
        let (a, srv) = adapter(vec![approval_script(available)]);
        let mut rx = a.subscribe().await.unwrap();
        a.create(create_req()).await.unwrap();
        a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
        let seen = until(&mut rx, is_permission).await;
        let id = request_id(&seen);
        assert_eq!(id, "i1");
        let deny = PermissionDecision::Deny {
            reason: Some("no".into()),
        };
        a.answer_permission(&radar(), answer(&id, deny.clone()))
            .await
            .unwrap();
        settle().await;
        assert_eq!(
            srv.answers()[0],
            json!({ "id": 7, "result": { "decision": want } })
        );
        let rest = until(&mut rx, is_idle).await;
        assert!(events(&rest).iter().any(|e| matches!(
            e.kind,
            EventKind::PermissionResolved {
                approved: false,
                ..
            }
        )));
        // A late second answer, even a different one, changes nothing.
        a.answer_permission(&radar(), answer(&id, PermissionDecision::ApproveOnce))
            .await
            .unwrap();
        assert_eq!(srv.answers().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn a_request_delivered_twice_is_one_request_answered_on_both_ids() {
    let mut script = approval_script(json!(["accept", "decline"]));
    // The server repeats the same approval under another JSON-RPC id.
    script[3].ins.push(server_request(
        8,
        "item/commandExecution/requestApproval",
        json!({ "threadId": "t1", "turnId": "u1", "itemId": "i1", "command": "rm -rf x" }),
    ));
    let (a, srv) = adapter(vec![script]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    let seen = until(&mut rx, is_permission).await;
    // Let the repeat arrive too.
    tokio::task::yield_now().await;
    a.answer_permission(&radar(), answer("i1", PermissionDecision::ApproveOnce))
        .await
        .unwrap();
    let rest = until(&mut rx, is_idle).await;
    let requests = events(&seen)
        .iter()
        .chain(events(&rest).iter())
        .filter(|e| matches!(e.kind, EventKind::PermissionRequested { .. }))
        .count();
    assert_eq!(requests, 1, "the dock sees one request");
    let ids: Vec<_> = srv.answers().iter().map(|m| m["id"].clone()).collect();
    assert!(ids.contains(&json!(7)), "{ids:?}");
}

#[tokio::test(start_paused = true)]
async fn a_file_change_approval_carries_the_diff_it_announced() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    s.push(request(
        "turn/start",
        vec![
            result(json!({ "turn": { "id": "u1" } })),
            note(
                "item/started",
                json!({ "threadId": "t1", "turnId": "u1", "item": { "type": "fileChange", "id": "f1", "status": "inProgress",
                    "changes": [{ "path": "a.txt", "kind": { "type": "add" }, "diff": "+hi" }] } }),
            ),
            server_request(3, "item/fileChange/requestApproval", json!({ "threadId": "t1", "turnId": "u1", "itemId": "f1" })),
        ],
    ));
    s.push(response(json!(3), vec![]));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    let seen = until(&mut rx, is_permission).await;
    let EventKind::PermissionRequested { request } = &events(&seen).last().unwrap().kind else {
        unreachable!()
    };
    assert_eq!(
        request.target,
        PermissionTarget::Write {
            path: "a.txt".into(),
            diff: Some("+hi".into())
        }
    );
    a.answer_permission(&radar(), answer("f1", PermissionDecision::ApproveOnce))
        .await
        .unwrap();
    settle().await;
    assert_eq!(
        srv.answers()[0],
        json!({ "id": 3, "result": { "decision": "accept" } })
    );
}

#[tokio::test(start_paused = true)]
async fn a_question_waits_for_the_operator_and_the_answers_go_back_by_question_id() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    let ask = |rpc: i64| {
        server_request(
            rpc,
            "item/tool/requestUserInput",
            json!({ "threadId": "t1", "turnId": "u1", "itemId": "q", "isBlocking": true,
                    "questions": [{ "id": "pick", "header": "Colour", "question": "Which colour?",
                        "options": [{ "label": "red", "description": "warm" }, { "label": "blue", "description": "" }],
                        "isOther": true, "isSecret": false }] }),
        )
    };
    s.push(request(
        "turn/start",
        vec![result(json!({ "turn": { "id": "u1" } })), ask(5), ask(6)],
    ));
    s.push(response(json!(5), vec![]));
    s.push(response(json!(6), vec![]));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    let seen = until(
        &mut rx,
        |u| matches!(u, Update::Event(e) if matches!(e.kind, EventKind::QuestionAsked { .. })),
    )
    .await;
    let EventKind::QuestionAsked { request } = &events(&seen).last().unwrap().kind else {
        unreachable!()
    };
    assert_eq!(request.request_id, "q");
    let q = &request.questions[0];
    assert_eq!((q.id.as_str(), q.header.as_deref(), q.text.as_str()), ("pick", Some("Colour"), "Which colour?"));
    assert_eq!(q.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["red", "blue"]);
    assert_eq!(q.choices[0].description.as_deref(), Some("warm"));
    assert!(q.free_text, "isOther lets the operator type an answer");
    settle().await;
    assert!(srv.answers().is_empty(), "nothing answers for the operator: {:?}", srv.answers());
    let reply = radar_sessions::QuestionReply { question_id: "pick".into(), values: vec!["red".into()], own_words: false };
    let answer = radar_sessions::QuestionAnswer {
        request_id: "q".into(),
        decision: radar_sessions::QuestionDecision::Answer { replies: vec![reply] },
    };
    a.answer_question(&radar(), answer.clone()).await.unwrap();
    a.answer_question(&radar(), answer).await.expect("a repeat is a no-op");
    settle().await;
    let want = json!({ "answers": { "pick": { "answers": ["red"] } } });
    let answers = srv.answers();
    // Delivered twice: one question in the dock, and each delivery gets the one answer.
    let asked = events(&seen).iter().filter(|e| matches!(e.kind, EventKind::QuestionAsked { .. })).count();
    assert_eq!(asked, 1);
    assert_eq!(answers, vec![json!({ "id": 5, "result": want }), json!({ "id": 6, "result": want })]);
}

#[tokio::test(start_paused = true)]
async fn a_request_radar_cannot_show_is_refused_and_said() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    s.push(request(
        "turn/start",
        vec![
            result(json!({ "turn": { "id": "u1" } })),
            server_request(
                5,
                "item/tool/call",
                json!({ "threadId": "t1", "turnId": "u1", "itemId": "q" }),
            ),
        ],
    ));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    let seen = until(
        &mut rx,
        |u| matches!(u, Update::Event(e) if matches!(e.kind, EventKind::Notice { .. })),
    )
    .await;
    assert!(
        matches!(&events(&seen).last().unwrap().kind, EventKind::Notice { text } if text.contains("item/tool/call"))
    );
    tokio::task::yield_now().await;
    let answers = srv.answers();
    assert_eq!(answers[0]["id"], 5);
    assert!(
        answers[0].get("error").is_some(),
        "refused, not answered: {answers:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_request_is_an_error_not_a_silent_ok() {
    let (a, _) = adapter(vec![approval_script(json!(["accept"]))]);
    a.create(create_req()).await.unwrap();
    let err = a
        .answer_permission(&radar(), answer("nope", PermissionDecision::ApproveOnce))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no pending"));
}

// ---- policy, steering, time -------------------------------------------------

#[tokio::test(start_paused = true)]
async fn a_reviewer_session_is_read_only_and_a_running_turn_is_steered() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    s.push(request(
        "turn/start",
        vec![
            result(json!({ "turn": { "id": "u1" } })),
            note(
                "turn/started",
                json!({ "threadId": "t1", "turn": { "id": "u1", "status": "inProgress" } }),
            ),
        ],
    ));
    s.push(request(
        "turn/steer",
        vec![result(json!({ "turnId": "u1" }))],
    ));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create_with(create_req(), Some(SessionPolicy::read_only()))
        .await
        .unwrap();
    assert_eq!(srv.sent("thread/start")[0]["sandbox"], "read-only");
    a.send(
        &radar(),
        SendInput {
            text: "first".into(),
        },
    )
    .await
    .unwrap();
    until(
        &mut rx,
        |u| matches!(u, Update::Event(e) if e.kind == EventKind::TurnStarted),
    )
    .await;
    a.send(
        &radar(),
        SendInput {
            text: "and also".into(),
        },
    )
    .await
    .unwrap();
    let steer = &srv.sent("turn/steer")[0];
    assert_eq!(steer["expectedTurnId"], "u1");
    assert_eq!(steer["input"][0]["text"], "and also");
}

#[tokio::test(start_paused = true)]
async fn a_silent_app_server_times_out_instead_of_hanging() {
    let (a, _) = adapter(vec![base()]);
    let err = a.create(create_req()).await.unwrap_err();
    assert!(err.to_string().contains("did not answer"), "{err}");
}

// ---- resume and reconcile ---------------------------------------------------

#[tokio::test(start_paused = true)]
async fn resume_replays_the_history_with_the_ids_the_live_events_had() {
    let mut script = load(PHASE2);
    let again = script
        .iter()
        .find(|s| matches!(&s.out, replay::Out::Request(m) if m == "thread/resume"))
        .unwrap()
        .clone();
    script.push(again);
    let (a, srv) = adapter(vec![script]);
    let mut rx = a.subscribe().await.unwrap();
    let req = ResumeSession {
        radar_session_id: radar(),
        provider_session_id: ProviderSessionId::new(THREAD),
        cwd: "/work/repo".into(),
        cursor: None,
        setup: SessionSetup {
            project_config: true,
            ..Default::default()
        },
    };
    a.resume(req.clone()).await.unwrap();
    let resume = &srv.sent("thread/resume")[0];
    assert!(
        resume.get("approvalPolicy").is_none() && resume.get("sandbox").is_none(),
        "a resumed thread takes the operator's config too: {resume}"
    );

    let mut first = Vec::new();
    while let Ok(u) = rx.try_recv() {
        first.push(u);
    }
    let ev = events(&first);
    let seqs: Vec<u64> = ev.iter().map(|e| e.sequence).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
    let ids = |ev: &[&Event]| -> Vec<String> {
        ev.iter()
            .filter(|e| e.durability == Durability::Persisted)
            .map(|e| e.provider_event_id.clone().unwrap().0)
            .collect()
    };
    let kinds: Vec<_> = ev.iter().map(|e| &e.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::UserMessage { text } if text.contains("ls -a")))
    );
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, EventKind::ToolCompleted { ok: true, .. }))
    );
    assert!(
        kinds.contains(&&EventKind::Aborted),
        "the interrupted turn is aborted"
    );
    let first_ids = ids(&ev);

    // Resuming again yields the same ids: the store drops the repeats.
    a.resume(req).await.unwrap();
    let mut second = Vec::new();
    while let Ok(u) = rx.try_recv() {
        second.push(u);
    }
    assert_eq!(ids(&events(&second)), first_ids);
    assert!(events(&second)[0].sequence > events(&first).last().unwrap().sequence);
}

#[tokio::test(start_paused = true)]
async fn a_lost_connection_is_reported_and_the_next_call_reconnects() {
    let mut p1 = approval_script(json!(["accept", "decline"]));
    p1[3].ins.push(close());
    let mut p2 = base();
    p2.push(request(
        "thread/resume",
        vec![result(json!({ "thread": { "id": "t1", "turns": [] } }))],
    ));
    p2.push(request(
        "turn/start",
        vec![result(json!({ "turn": { "id": "u2" } }))],
    ));
    let (a, srv) = adapter(vec![p1, p2]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    let seen = until(&mut rx, |u| matches!(u, Update::Disconnected { .. })).await;
    // The request that can never be answered now is closed, not left dangling.
    assert!(events(&seen).iter().any(|e| matches!(
        e.kind,
        EventKind::PermissionResolved {
            approved: false,
            ..
        }
    )));
    let err = a
        .answer_permission(&radar(), answer("i1", PermissionDecision::ApproveOnce))
        .await;
    assert!(err.is_err() || srv.answers().is_empty());
    assert!(
        srv.answers().is_empty(),
        "nothing was sent to a dead process"
    );

    // While it is down a send fails loudly; the adapter brings it back by itself.
    assert!(
        a.send(
            &radar(),
            SendInput {
                text: "too soon".into()
            }
        )
        .await
        .is_err()
    );
    until(&mut rx, |u| matches!(u, Update::Reconnected)).await;
    a.send(
        &radar(),
        SendInput {
            text: "again".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(*srv.spawns.lock().unwrap(), 2);
    let resumed = &srv.sent("thread/resume")[0];
    assert_eq!(resumed["threadId"], "t1");
    assert!(resumed.get("approvalPolicy").is_none(), "{resumed}");
}

// ---- sessions ---------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn sessions_list_and_delete_through_the_provider() {
    let mut s = base();
    s.push(request(
        "thread/list",
        vec![result(json!({ "data": [
            { "id": "a", "preview": "first", "cwd": "/x", "updatedAt": 5 },
            { "id": "b", "name": "named", "preview": "p", "cwd": "/y", "updatedAt": 6 }
        ] }))],
    ));
    s.push(request("thread/delete", vec![result(json!({}))]));
    let (a, srv) = adapter(vec![s]);
    let list = a.list_sessions().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].title.as_deref(), Some("named"));
    a.delete_session(&ProviderSessionId::new("a"))
        .await
        .unwrap();
    assert_eq!(srv.sent("thread/delete")[0]["threadId"], "a");
}

/// A session whose `skills/list` answers with one skill, then `calls`.
fn commands_script(calls: Vec<Seg>) -> Vec<Seg> {
    let mut s = base();
    s.push(request("thread/start", vec![result(json!({ "thread": { "id": "t1" }, "model": "gpt-test" }))]));
    s.push(request(
        "skills/list",
        vec![result(json!({ "data": [{ "cwd": "/work/repo", "errors": [], "skills": [
            { "name": "pdf:pdf", "description": "Work with PDF files. More.", "enabled": true, "path": "/p/pdf/SKILL.md", "scope": "user" }
        ] }] }))],
    ));
    s.extend(calls);
    s
}

#[tokio::test]
async fn the_menu_has_compact_and_review_at_once_and_the_skills_when_codex_lists_them() {
    let (a, _srv) = adapter(vec![commands_script(vec![])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    let seen = until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.iter().any(|c| c.name == "pdf:pdf"))).await;
    let lists: Vec<Vec<String>> = seen
        .iter()
        .filter_map(|u| match u {
            Update::Commands { session, commands } => {
                assert_eq!(session, &radar());
                Some(commands.iter().map(|c| c.name.clone()).collect())
            }
            _ => None,
        })
        .collect();
    assert_eq!(lists, vec![vec!["compact", "review"], vec!["compact", "review", "pdf:pdf"]]);
    let Some(Update::Commands { commands, .. }) = seen.last() else { panic!("the skills arrived last") };
    assert_eq!(commands[2].description, "Work with PDF files.");
}

#[tokio::test]
async fn compact_is_the_compaction_call_not_a_sentence_for_the_model() {
    let (a, srv) = adapter(vec![commands_script(vec![request("thread/compact/start", vec![result(json!({}))])])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.len() == 3)).await;
    a.send(&radar(), SendInput { text: "/compact".into() }).await.unwrap();
    assert_eq!(srv.sent("thread/compact/start"), vec![json!({ "threadId": "t1" })]);
    assert!(srv.sent("turn/start").is_empty(), "nothing went to the model as text");
}

#[tokio::test]
async fn review_looks_at_the_uncommitted_changes_or_at_what_the_text_asks() {
    let review = |id: &str| request("review/start", vec![result(json!({ "turn": { "id": id }, "reviewThreadId": "t1" }))]);
    let (a, srv) = adapter(vec![commands_script(vec![review("u1")])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.len() == 3)).await;
    a.send(&radar(), SendInput { text: "/review".into() }).await.unwrap();
    assert_eq!(srv.sent("review/start"), vec![json!({ "threadId": "t1", "target": { "type": "uncommittedChanges" }, "delivery": "inline" })]);

    let (a, srv) = adapter(vec![commands_script(vec![review("u1")])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.len() == 3)).await;
    a.send(&radar(), SendInput { text: "/review focus on the error paths".into() }).await.unwrap();
    assert_eq!(srv.sent("review/start")[0]["target"], json!({ "type": "custom", "instructions": "focus on the error paths" }));
}

#[tokio::test]
async fn a_skill_is_a_turn_whose_input_is_the_skill_and_the_text_after_it() {
    let turn = request("turn/start", vec![result(json!({ "turn": { "id": "u1" } }))]);
    let (a, srv) = adapter(vec![commands_script(vec![turn])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.len() == 3)).await;
    a.send(&radar(), SendInput { text: "/pdf:pdf merge a and b".into() }).await.unwrap();
    assert_eq!(
        srv.sent("turn/start")[0]["input"],
        json!([{ "type": "skill", "name": "pdf:pdf", "path": "/p/pdf/SKILL.md" }, { "type": "text", "text": "merge a and b" }])
    );
}

#[tokio::test]
async fn text_that_is_not_one_of_the_sessions_commands_is_sent_as_written() {
    let turn = || request("turn/start", vec![result(json!({ "turn": { "id": "u1" } }))]);
    let (a, srv) = adapter(vec![commands_script(vec![turn()])]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Commands { commands, .. } if commands.len() == 3)).await;
    a.send(&radar(), SendInput { text: "/etc/hosts has a typo".into() }).await.unwrap();
    assert_eq!(srv.sent("turn/start")[0]["input"], json!([{ "type": "text", "text": "/etc/hosts has a typo" }]));
    assert!(srv.sent("review/start").is_empty() && srv.sent("thread/compact/start").is_empty());
}

#[test]
fn the_login_command_is_codex_own_and_holds_no_secret() {
    let (program, args) = crate::login_command();
    assert_eq!(program, "codex");
    assert_eq!(args, ["login", "--device-auth"]);
}

#[test]
fn account_shapes_map_to_who_is_signed_in() {
    use crate::auth_from_account as f;
    assert_eq!(
        f(&json!({"account":{"type":"apiKey"},"requiresOpenaiAuth":true})),
        AuthStatus::SignedIn {
            as_user: "API key".into()
        }
    );
    assert_eq!(
        f(&json!({"account":null,"requiresOpenaiAuth":true})),
        AuthStatus::SignedOut
    );
    assert_eq!(
        f(&json!({"account":null,"requiresOpenaiAuth":false})),
        AuthStatus::Unknown
    );
}

// ---- session setup ----------------------------------------------------------

fn full_setup() -> SessionSetup {
    SessionSetup {
        extra_dirs: vec!["/a/lib".into()],
        agent: Some("reviewer".into()),
        skill_dirs: vec!["/s/skills".into()],
        skills: vec!["demo".into(), "off".into(), "gone".into()],
        instruction_dirs: vec!["/i".into()],
        plugin_dirs: vec!["/p".into()],
        project_config: false,
        local_only: true,
        review: false,
        tool_server: None,
    }
}

fn notices(rx: &mut Updates) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(u) = rx.try_recv() {
        if let Update::Event(Event {
            kind: EventKind::Notice { text },
            ..
        }) = u
        {
            out.push(text);
        }
    }
    out
}

/// The methods the client called, in order.
fn methods(srv: &ReplaySpawner) -> Vec<String> {
    srv.log
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter_map(|m| m.get("method").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_setup_is_sent_as_writable_roots_skill_folders_and_enabled_skills() {
    let mut s = base();
    s.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    s.push(request(
        "skills/list",
        vec![result(json!({ "data": [{
            "cwd": "/work/repo",
            "errors": [],
            "skills": [
                { "name": "demo", "description": "d", "path": "/s/skills/demo/SKILL.md", "scope": "user", "enabled": true, "pluginId": null },
                { "name": "off", "description": "o", "path": "/s/skills/off/SKILL.md", "scope": "user", "enabled": false, "pluginId": null },
            ],
        }]}))],
    ));
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(CreateSession {
        setup: full_setup(),
        ..create_req()
    })
    .await
    .unwrap();

    assert_eq!(srv.sent("skills/extraRoots/set"), [json!({ "extraRoots": ["/s/skills"] })]);
    assert_eq!(
        srv.sent("skills/list"),
        [json!({ "cwds": ["/work/repo"], "forceReload": true })]
    );
    assert!(srv.sent("skills/config/write").is_empty(), "radar never writes Codex's own config");
    // The policy is the operator's own Codex config, never the setup's; the roots are
    // named for a workspace-write sandbox, the only one that reads them.
    assert_eq!(
        srv.sent("thread/start"),
        [json!({
            "cwd": "/work/repo",
            "config": {
                "sandbox_workspace_write": { "writable_roots": ["/a/lib"] },
                "project_doc_max_bytes": 0,
            },
        })]
    );
    let order = methods(&srv);
    let at = |m: &str| order.iter().position(|x| x == m).unwrap();
    assert!(at("skills/extraRoots/set") < at("skills/list"));
    assert!(at("skills/list") < at("thread/start"), "the skills are checked before the thread starts");

    let said = notices(&mut rx).join("\n");
    assert!(said.contains("`off` is switched off in Codex's own config"), "{said}");
    assert!(said.contains("no skill named `gone`"), "{said}");
    assert!(said.contains("`reviewer` was not applied"), "{said}");
    assert!(said.contains("instruction folders"), "{said}");
    assert!(said.contains("plugin folders"), "{said}");
    assert!(!said.contains("Project configuration"), "off is applied, not a notice: {said}");
    assert!(!said.contains("demo"), "a skill that is there says nothing: {said}");
}

#[tokio::test(start_paused = true)]
async fn warm_starts_the_app_server_and_an_unused_thread_is_unloaded_on_discard() {
    let mut s = base();
    s.push(request("thread/start", vec![result(json!({ "thread": { "id": "t1" }, "approvalPolicy": "on-request", "sandbox": { "type": "readOnly" } }))]));
    s.push(request("thread/unsubscribe", vec![result(json!({ "status": "unsubscribed" }))]));
    let (a, srv) = adapter(vec![s]);
    assert!(a.idle_session_is_free(), "a thread nobody writes to is not in Codex's list");
    a.warm().await.unwrap();
    settle().await;
    assert_eq!(methods(&srv), ["initialize", "initialized"], "the app-server only, no thread");
    let _rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.discard(&radar()).await.unwrap();
    assert_eq!(srv.sent("thread/unsubscribe"), [json!({ "threadId": "t1" })]);
    // Forgotten here too: nothing can be sent to the thread any more.
    assert!(a.send(&radar(), SendInput { text: "hi".into() }).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_plain_setup_sends_only_the_folder_and_codexs_own_config_decides() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" }, "approvalPolicy": "on-request", "sandbox": { "type": "readOnly" } }))],
    ));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    assert_eq!(srv.sent("thread/start"), [json!({ "cwd": "/work/repo" })]);
    assert_eq!(methods(&srv), ["initialize", "initialized", "thread/start"]);
    assert!(notices(&mut rx).is_empty(), "a config that asks says nothing");
}

#[tokio::test(start_paused = true)]
async fn a_config_that_never_asks_is_the_operators_and_the_session_says_so() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" }, "approvalPolicy": "never", "sandbox": { "type": "dangerFullAccess" } }))],
    ));
    let (a, srv) = adapter(vec![s]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    assert_eq!(srv.sent("thread/start"), [json!({ "cwd": "/work/repo" })]);
    let said = notices(&mut rx).join("\n");
    assert!(said.contains("approval policy `never`") && said.contains("dangerFullAccess"), "{said}");
}

#[tokio::test(start_paused = true)]
async fn a_review_session_keeps_ggs_strict_policy_over_the_operators_config() {
    let mut s = base();
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" }, "approvalPolicy": "untrusted" }))],
    ));
    let (a, srv) = adapter(vec![s]);
    a.create(CreateSession { setup: SessionSetup { review: true, project_config: true, ..Default::default() }, ..create_req() })
        .await
        .unwrap();
    assert_eq!(
        srv.sent("thread/start"),
        [json!({ "cwd": "/work/repo", "approvalPolicy": "untrusted", "sandbox": "workspace-write" })]
    );
    // And radar's policy for every session when the operator's is turned off.
    let mut s = base();
    s.push(request("thread/start", vec![result(json!({ "thread": { "id": "t2" } }))]));
    let spawner = ReplaySpawner::new(vec![s]);
    let cfg = CodexConfig { user_policy: false, clock: Arc::new(|| 1234), ..CodexConfig::default() };
    let b = CodexAdapter::with(Box::new(Arc::clone(&spawner)), cfg);
    b.create(create_req()).await.unwrap();
    assert_eq!(spawner.sent("thread/start")[0]["approvalPolicy"], "untrusted");
}

#[tokio::test(start_paused = true)]
async fn a_review_sandbox_gets_no_writable_roots() {
    let mut s = base();
    s.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    s.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    let (a, srv) = adapter(vec![s]);
    a.create_with(
        CreateSession {
            setup: SessionSetup {
                extra_dirs: vec!["/a/lib".into()],
                skill_dirs: vec!["/s/skills".into()],
                project_config: true,
                ..Default::default()
            },
            ..create_req()
        },
        Some(SessionPolicy::read_only()),
    )
    .await
    .unwrap();
    assert_eq!(
        srv.sent("thread/start"),
        [json!({ "cwd": "/work/repo", "approvalPolicy": "untrusted", "sandbox": "read-only" })]
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_app_server_gets_the_skill_folders_and_the_setup_again_on_resume() {
    let mut p1 = base();
    p1.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    p1.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    p1.push(request(
        "turn/start",
        vec![result(json!({ "turn": { "id": "u1" } })), close()],
    ));
    let mut p2 = base();
    p2.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    p2.push(request(
        "thread/resume",
        vec![result(json!({ "thread": { "id": "t1", "turns": [] } }))],
    ));
    p2.push(request(
        "turn/start",
        vec![result(json!({ "turn": { "id": "u2" } }))],
    ));
    let (a, srv) = adapter(vec![p1, p2]);
    let mut rx = a.subscribe().await.unwrap();
    let setup = SessionSetup {
        extra_dirs: vec!["/a/lib".into()],
        skill_dirs: vec!["/s/skills".into()],
        project_config: true,
        ..Default::default()
    };
    a.create(CreateSession {
        setup: setup.clone(),
        ..create_req()
    })
    .await
    .unwrap();
    a.send(&radar(), SendInput { text: "x".into() }).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Disconnected { .. })).await;
    until(&mut rx, |u| matches!(u, Update::Reconnected)).await;
    a.send(&radar(), SendInput { text: "again".into() }).await.unwrap();

    assert_eq!(
        srv.sent("skills/extraRoots/set"),
        [json!({ "extraRoots": ["/s/skills"] }), json!({ "extraRoots": ["/s/skills"] })]
    );
    assert_eq!(
        srv.sent("thread/resume"),
        [json!({
            "threadId": "t1",
            "cwd": "/work/repo",
            "config": { "sandbox_workspace_write": { "writable_roots": ["/a/lib"] } },
        })]
    );
}

#[tokio::test(start_paused = true)]
async fn resume_takes_the_setup_too() {
    let mut s = base();
    s.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    s.push(request(
        "thread/resume",
        vec![result(json!({ "thread": { "id": THREAD, "turns": [] } }))],
    ));
    let (a, srv) = adapter(vec![s]);
    a.resume(ResumeSession {
        radar_session_id: radar(),
        provider_session_id: ProviderSessionId::new(THREAD),
        cwd: "/work/repo".into(),
        cursor: None,
        setup: full_setup(),
    })
    .await
    .unwrap();
    assert_eq!(srv.sent("skills/extraRoots/set"), [json!({ "extraRoots": ["/s/skills"] })]);
    assert_eq!(
        srv.sent("thread/resume"),
        [json!({
            "threadId": THREAD,
            "cwd": "/work/repo",
            "config": {
                "sandbox_workspace_write": { "writable_roots": ["/a/lib"] },
                "project_doc_max_bytes": 0,
            },
        })]
    );
}

#[tokio::test(start_paused = true)]
async fn the_catalog_is_skills_list_for_the_setup_and_leaves_the_roots_as_they_were() {
    let mut s = base();
    s.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    s.push(request(
        "skills/list",
        vec![result(json!({ "data": [{
            "cwd": "/work/repo",
            "errors": [],
            "skills": [
                { "name": "review", "description": "Review a diff", "path": "/s/skills/review/SKILL.md", "scope": "user", "enabled": true, "pluginId": null },
                { "name": "imagegen", "description": "Make images", "path": "/home/u/.codex/skills/.system/imagegen/SKILL.md", "scope": "system", "enabled": true, "pluginId": null },
            ],
        }]}))],
    ));
    s.push(request("skills/extraRoots/set", vec![result(json!({}))]));
    let (a, srv) = adapter(vec![s]);
    let got = a
        .catalog(
            "/work/repo".into(),
            SessionSetup {
                skill_dirs: vec!["/s/skills".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(got.agents.is_empty());
    let skills: Vec<_> = got
        .skills
        .iter()
        .map(|e| (e.name.as_str(), e.description.as_deref(), e.source.as_deref()))
        .collect();
    assert_eq!(
        skills,
        [
            ("imagegen", Some("Make images"), Some("system")),
            ("review", Some("Review a diff"), Some("/s/skills")),
        ]
    );
    assert_eq!(
        srv.sent("skills/extraRoots/set"),
        [json!({ "extraRoots": ["/s/skills"] }), json!({ "extraRoots": [] })]
    );
    assert_eq!(srv.sent("skills/list"), [json!({ "cwds": ["/work/repo"], "forceReload": true })]);
}

#[test]
fn skills_are_listed_and_agents_are_not() {
    let caps = crate::capabilities();
    assert!(caps.skills);
    assert!(!caps.agents);
    let controls: Vec<_> = caps.unsupported.iter().map(|u| u.control.as_str()).collect();
    for c in ["agents", "instruction folders", "plugin folders"] {
        assert!(controls.contains(&c), "{c} is said to be unsupported");
    }
}

fn provider(exe: &str, home: &str) -> ProviderSetup {
    ProviderSetup {
        executable: Some(exe.into()),
        config_dir: Some(home.into()),
    }
}

#[tokio::test(start_paused = true)]
async fn a_new_executable_or_home_restarts_the_app_server_on_the_next_use() {
    let mut p1 = base();
    p1.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    let mut p2 = base();
    p2.push(request("model/list", vec![result(json!({ "data": [] }))]));
    let (a, srv) = adapter(vec![p1, p2]);
    a.create(create_req()).await.unwrap();
    assert_eq!(*srv.spawns.lock().unwrap(), 1);

    a.configure(provider("/opt/codex", "/h"));
    a.configure(provider("/opt/codex", "/h"));
    assert_eq!(
        *srv.configured.lock().unwrap(),
        [provider("/opt/codex", "/h")],
        "the same setting twice is one change"
    );
    assert_eq!(*srv.spawns.lock().unwrap(), 1, "nothing restarts until it is used");

    a.list_models().await.unwrap();
    assert_eq!(*srv.spawns.lock().unwrap(), 2);
    // Not restarted again by the same change.
    assert!(a.list_models().await.is_err(), "the script has one model/list per process");
    assert_eq!(*srv.spawns.lock().unwrap(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_app_server_is_not_restarted_under_a_running_turn() {
    let mut p1 = base();
    p1.push(request(
        "thread/start",
        vec![result(json!({ "thread": { "id": "t1" } }))],
    ));
    p1.push(request(
        "turn/start",
        vec![
            result(json!({ "turn": { "id": "u1" } })),
            note("turn/started", json!({ "threadId": "t1", "turn": { "id": "u1", "status": "inProgress" } })),
        ],
    ));
    p1.push(request("model/list", vec![result(json!({ "data": [] }))]));
    let mut p2 = base();
    p2.push(request("model/list", vec![result(json!({ "data": [] }))]));
    let (a, srv) = adapter(vec![p1, p2]);
    let mut rx = a.subscribe().await.unwrap();
    a.create(create_req()).await.unwrap();
    a.send(&radar(), SendInput { text: "go".into() }).await.unwrap();
    until(&mut rx, |u| matches!(u, Update::Event(e) if e.kind == EventKind::TurnStarted)).await;

    a.configure(provider("/opt/codex", "/h"));
    a.list_models().await.unwrap();
    assert_eq!(*srv.spawns.lock().unwrap(), 1, "a turn is running");

    srv.inject(vec![note(
        "turn/completed",
        json!({ "threadId": "t1", "turn": { "id": "u1", "status": "completed", "items": [] } }),
    )]);
    until(&mut rx, is_idle).await;
    a.list_models().await.unwrap();
    assert_eq!(*srv.spawns.lock().unwrap(), 2, "between turns it starts again");
}
