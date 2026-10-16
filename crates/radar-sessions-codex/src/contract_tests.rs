//! radar-sessions' shared contract kit, run against this adapter over a scripted
//! `codex app-server` that acts out each scenario with real protocol shapes
//! (the shapes come from the recording in tests/fixtures).

use std::sync::Arc;
use std::time::Duration;

use radar_sessions::contract::{self, Harness, Scenario};
use radar_sessions::{Adapter, BoxFuture};
use serde_json::{Value, json};

use crate::replay::{
    ReplaySpawner, Seg, answer, note, notify, request, response, result, server_request,
};
use crate::{CodexAdapter, CodexConfig};

const T: &str = "t1";

fn item_note(method: &str, turn: &str, item: Value) -> Value {
    note(
        method,
        json!({ "threadId": T, "turnId": turn, "item": item }),
    )
}

fn user_item(n: u32, text: &str) -> Value {
    json!({ "type": "userMessage", "id": format!("um{n}"), "content": [{ "type": "text", "text": text }] })
}

fn agent_item(id: &str, text: &str) -> Value {
    json!({ "type": "agentMessage", "id": id, "text": text, "phase": "final_answer" })
}

fn cmd_item(id: &str, status: &str, exit: Option<i64>) -> Value {
    json!({ "type": "commandExecution", "id": id, "command": "ls", "cwd": "/tmp", "status": status,
            "exitCode": exit, "aggregatedOutput": "a\n" })
}

fn turn_started(n: u32) -> Value {
    note(
        "turn/started",
        json!({ "threadId": T, "turn": { "id": format!("u{n}"), "status": "inProgress", "items": [] } }),
    )
}

fn turn_completed(n: u32, status: &str) -> Value {
    note(
        "turn/completed",
        json!({ "threadId": T, "turn": { "id": format!("u{n}"), "status": status, "items": [] } }),
    )
}

fn started_result(n: u32) -> Value {
    result(json!({ "turn": { "id": format!("u{n}"), "status": "inProgress", "items": [] } }))
}

/// A user message, then a streamed answer, as live notifications.
fn streamed(n: u32, text: &str, answer_id: &str, answer: &str) -> Vec<Value> {
    vec![
        item_note("item/completed", &format!("u{n}"), user_item(n, text)),
        item_note(
            "item/started",
            &format!("u{n}"),
            json!({ "type": "agentMessage", "id": answer_id, "text": "", "phase": "final_answer" }),
        ),
        note(
            "item/agentMessage/delta",
            json!({ "threadId": T, "turnId": format!("u{n}"), "itemId": answer_id, "delta": "part " }),
        ),
        note(
            "item/agentMessage/delta",
            json!({ "threadId": T, "turnId": format!("u{n}"), "itemId": answer_id, "delta": "of it" }),
        ),
        item_note(
            "item/completed",
            &format!("u{n}"),
            agent_item(answer_id, answer),
        ),
    ]
}

fn approval(rpc: i64, n: u32) -> Value {
    server_request(
        rpc,
        "item/commandExecution/requestApproval",
        json!({ "threadId": T, "turnId": format!("u{n}"), "itemId": "c1", "command": "ls",
                "commandActions": [{ "type": "listFiles", "command": "ls" }],
                "availableDecisions": ["accept", "acceptForSession", "decline", "cancel"] }),
    )
}

fn resolved(rpc: i64) -> Value {
    note(
        "serverRequest/resolved",
        json!({ "threadId": T, "requestId": rpc }),
    )
}

/// A finished turn as `thread/resume` returns it.
fn history_turn(n: u32, status: &str, items: Vec<Value>) -> Value {
    json!({ "id": format!("u{n}"), "status": status, "itemsView": "full", "items": items })
}

fn base(history: Vec<Value>, resumes: usize) -> Vec<Seg> {
    let mut s = vec![
        request("initialize", vec![result(json!({ "userAgent": "test" }))]),
        notify("initialized"),
        request(
            "account/read",
            vec![result(
                json!({ "account": { "type": "chatgpt", "email": "operator@example.com", "planType": "plus" }, "requiresOpenaiAuth": true }),
            )],
        ),
        request(
            "model/list",
            vec![result(
                json!({ "data": [{ "id": "gpt-test", "displayName": "GPT Test", "hidden": false }] }),
            )],
        ),
        request(
            "thread/start",
            vec![result(
                json!({ "thread": { "id": T }, "model": "gpt-test" }),
            )],
        ),
    ];
    for _ in 0..resumes {
        s.push(request(
            "thread/resume",
            vec![result(
                json!({ "thread": { "id": T, "turns": history }, "model": "gpt-test" }),
            )],
        ));
    }
    s
}

struct CodexHarness {
    spawner: Arc<ReplaySpawner>,
    /// What the provider's disk holds: survives a restart.
    history: Vec<Value>,
    scenario: Scenario,
    released: bool,
}

impl CodexHarness {
    fn new() -> Self {
        Self {
            spawner: ReplaySpawner::new(vec![]),
            history: vec![],
            scenario: Scenario::Text,
            released: false,
        }
    }

    fn adapter_over(spawner: &Arc<ReplaySpawner>) -> Box<dyn Adapter> {
        let cfg = CodexConfig {
            clock: Arc::new(|| 1000),
            ..CodexConfig::default()
        };
        Box::new(CodexAdapter::with(Box::new(Arc::clone(spawner)), cfg))
    }

    /// The segments that act out `scenario`'s turns.
    fn turns(scenario: Scenario) -> (Vec<Seg>, Vec<Value>) {
        let mut segs = Vec::new();
        let mut history = Vec::new();
        match scenario {
            Scenario::Text | Scenario::TwoTurns => {
                let count = if scenario == Scenario::Text { 1 } else { 2 };
                for n in 1..=count {
                    let text = if n == 1 {
                        "Hello there"
                    } else {
                        "Second answer"
                    };
                    let mut ins = vec![started_result(n), turn_started(n)];
                    ins.extend(streamed(n, "hi", &format!("am{n}"), text));
                    ins.push(turn_completed(n, "completed"));
                    segs.push(request("turn/start", ins));
                    history.push(history_turn(
                        n,
                        "completed",
                        vec![user_item(n, "hi"), agent_item(&format!("am{n}"), text)],
                    ));
                }
            }
            Scenario::Tool => {
                segs.push(request(
                    "turn/start",
                    vec![
                        started_result(1),
                        turn_started(1),
                        item_note("item/completed", "u1", user_item(1, "run something")),
                        item_note("item/started", "u1", cmd_item("c1", "inProgress", None)),
                        approval(1, 1),
                    ],
                ));
                let mut yes = vec![
                    resolved(1),
                    item_note("item/completed", "u1", cmd_item("c1", "completed", Some(0))),
                ];
                yes.push(item_note(
                    "item/completed",
                    "u1",
                    agent_item("am1", "Ran it"),
                ));
                yes.push(turn_completed(1, "completed"));
                segs.push(answer(json!(1), "acceptForSession", yes.clone()));
                segs.push(answer(json!(1), "accept", yes));
                let mut no = vec![
                    resolved(1),
                    item_note("item/completed", "u1", cmd_item("c1", "declined", None)),
                ];
                no.push(item_note(
                    "item/completed",
                    "u1",
                    agent_item("am1", "Understood"),
                ));
                no.push(turn_completed(1, "completed"));
                segs.push(answer(json!(1), "decline", no));
            }
            Scenario::Redelivery => {
                let um = item_note("item/completed", "u1", user_item(1, "go"));
                let once = item_note("item/completed", "u1", agent_item("am1", "Once"));
                segs.push(request(
                    "turn/start",
                    vec![
                        started_result(1),
                        turn_started(1),
                        um.clone(),
                        um,
                        once.clone(),
                        once,
                        item_note("item/started", "u1", cmd_item("c1", "inProgress", None)),
                        approval(1, 1),
                        approval(2, 1),
                    ],
                ));
                segs.push(answer(
                    json!(1),
                    "accept",
                    vec![
                        resolved(1),
                        resolved(2),
                        item_note("item/completed", "u1", cmd_item("c1", "completed", Some(0))),
                        item_note("item/completed", "u1", agent_item("am2", "Twice delivered")),
                        item_note("item/completed", "u1", agent_item("am2", "Twice delivered")),
                        turn_completed(1, "completed"),
                    ],
                ));
            }
            Scenario::LongTurn => {
                let mut ins = vec![started_result(1), turn_started(1)];
                ins.extend(streamed(1, "keep going", "am1", "Working on it"));
                segs.push(request("turn/start", ins));
                segs.push(request(
                    "turn/interrupt",
                    vec![result(json!({})), turn_completed(1, "interrupted")],
                ));
                history.push(history_turn(
                    1,
                    "inProgress",
                    vec![
                        user_item(1, "keep going"),
                        agent_item("am1", "Working on it"),
                    ],
                ));
            }
            Scenario::Reordered => {}
            Scenario::Question => {
                segs.push(request(
                    "turn/start",
                    vec![
                        started_result(1),
                        turn_started(1),
                        item_note("item/completed", "u1", user_item(1, "ask me something")),
                        server_request(
                            1,
                            "item/tool/requestUserInput",
                            json!({ "threadId": T, "turnId": "u1", "itemId": "q1", "isBlocking": true,
                                    "questions": [{ "id": "colour", "header": "Colour", "question": "Which colour?",
                                        "options": [{ "label": "red", "description": "" }, { "label": "blue", "description": "" }],
                                        "isOther": true }] }),
                        ),
                    ],
                ));
                segs.push(response(
                    json!(1),
                    vec![
                        resolved(1),
                        item_note("item/completed", "u1", agent_item("am1", "Red it is")),
                        turn_completed(1, "completed"),
                    ],
                ));
            }
        }
        (segs, history)
    }
}

impl Harness for CodexHarness {
    fn adapter(&mut self, scenario: Scenario) -> BoxFuture<'_, Box<dyn Adapter>> {
        Box::pin(async move {
            let (turns, history) = Self::turns(scenario);
            self.scenario = scenario;
            self.released = false;
            // A TwoTurns restart finds only the first turn on disk.
            self.history = if scenario == Scenario::TwoTurns {
                history[..1].to_vec()
            } else {
                history
            };
            let mut script = base(vec![], 0);
            script.extend(turns);
            self.spawner = ReplaySpawner::new(vec![script]);
            Self::adapter_over(&self.spawner)
        })
    }

    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>> {
        Box::pin(async move {
            self.spawner.kill();
            // The new process holds the thread on disk and acts out the next turn.
            let (turns, _) = Self::turns(self.scenario);
            let mut script = base(self.history.clone(), 3);
            script.extend(turns.into_iter().skip(self.history.len()));
            self.spawner = ReplaySpawner::new(vec![script]);
            Self::adapter_over(&self.spawner)
        })
    }

    fn settle(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {
            for _ in 0..200 {
                tokio::task::yield_now().await;
            }
        })
    }

    fn disconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.spawner
                .down
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.spawner.kill();
        })
    }

    fn reconnect(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // The new process finds the thread as the disk holds it.
            let mut history = self.history.clone();
            if self.released {
                history = vec![history_turn(
                    1,
                    "completed",
                    vec![
                        user_item(1, "keep going"),
                        agent_item("am1", "Working on it"),
                        agent_item("am2", "Finished"),
                    ],
                )];
            }
            self.spawner.push_script(base(history, 2));
            self.spawner
                .down
                .store(false, std::sync::atomic::Ordering::SeqCst);
            // Let the adapter's backoff elapse (the clock is paused).
            tokio::time::advance(Duration::from_secs(60)).await;
        })
    }

    fn release(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.released = true;
            // Connected: the provider says so itself.
            self.spawner.inject(vec![
                item_note("item/completed", "u1", agent_item("am2", "Finished")),
                turn_completed(1, "completed"),
            ]);
        })
    }

    fn skips(&self) -> Vec<(Scenario, &'static str)> {
        vec![(
            Scenario::Reordered,
            "app-server notifications carry no id or sequence, so the adapter's order is the provider's",
        )]
    }
}

#[tokio::test(start_paused = true)]
async fn the_shared_contract_holds() {
    let mut h = CodexHarness::new();
    let report = contract::run_all(&mut h).await;
    for name in [
        "lifecycle",
        "stream",
        "duplicates",
        "approve",
        "approve_for_session",
        "deny",
        "question",
        "stop",
        "resume",
        "resume_waiting",
        "lost_connection",
    ] {
        assert!(report.ran.contains(&name), "{name} did not run: {report:?}");
    }
    assert!(
        report.skipped.iter().all(|(n, _)| *n == "out_of_order"),
        "{report:?}"
    );
}
