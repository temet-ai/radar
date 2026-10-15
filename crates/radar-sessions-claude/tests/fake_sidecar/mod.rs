//! A scripted stand-in for the Node sidecar, speaking its protocol (sidecars/claude-agent/README.md)
//! and acting out the contract kit's scenarios. The provider's state (sessions and their persisted
//! history) lives in [`Provider`], shared by every "process" the adapter starts, so a restart or a
//! lost connection leaves the history behind like Claude Code's own transcript does.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use radar_sessions::contract::Scenario;
use radar_sessions::BoxFuture;
use radar_sessions_claude::{Spawned, Spawner};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

#[derive(Default)]
pub struct Hist {
    /// Persisted lines, without `seq`.
    pub events: Vec<Value>,
    pub next: u64,
    pub turns: usize,
    pub running: bool,
    pub pending: Option<String>,
    pub answered: HashSet<String>,
}

pub struct Provider {
    pub scenario: Scenario,
    pub sessions: HashMap<String, Hist>,
    pub connected: bool,
    /// Bumped to end every running process (a lost connection).
    pub epoch: watch::Sender<u64>,
    /// Set by `release()`; a hanging turn finishes.
    pub release: watch::Sender<u64>,
    /// Every request the adapter wrote, in order.
    pub requests: Vec<Value>,
}

pub type Shared = Arc<Mutex<Provider>>;

pub fn provider(scenario: Scenario) -> Shared {
    Arc::new(Mutex::new(Provider {
        scenario,
        sessions: HashMap::new(),
        connected: true,
        epoch: watch::channel(0).0,
        release: watch::channel(0).0,
        requests: vec![],
    }))
}

pub struct FakeSpawner(pub Shared);

impl Spawner for FakeSpawner {
    fn spawn(&self) -> BoxFuture<'_, Result<Spawned>> {
        let shared = Arc::clone(&self.0);
        Box::pin(async move {
            if !shared.lock().unwrap().connected {
                return Err(anyhow!("the provider is unreachable"));
            }
            let (tx, lines) = mpsc::unbounded_channel::<String>();
            let (writer, mut wrx) = mpsc::unbounded_channel::<String>();
            let mut epoch = shared.lock().unwrap().epoch.subscribe();
            let mut release = shared.lock().unwrap().release.subscribe();
            let mut proc = Proc { shared, tx, seq: HashMap::new() };
            proc.say_line(json!({"type":"ready","protocol":1,"sidecar_version":"0.1.0","sdk_version":"0.3.296"}));
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        req = wrx.recv() => {
                            let Some(req) = req else { return };
                            proc.handle(serde_json::from_str(&req).unwrap());
                        }
                        _ = epoch.changed() => return,
                        _ = release.changed() => proc.released(),
                    }
                }
            });
            Ok(Spawned { lines, writer, stderr_tail: Arc::default() })
        })
    }
}

struct Proc {
    shared: Shared,
    tx: mpsc::UnboundedSender<String>,
    /// Per session: lines emitted by this process.
    seq: HashMap<String, u64>,
}

fn line(radar: &str, pid: &str, durability: &str, kind: &str, id: Option<&str>, payload: Value) -> Value {
    json!({"type":"event","session":radar,"provider_session_id":pid,"provider_event_id":id,"durability":durability,"kind":kind,"payload":payload})
}

impl Proc {
    fn say_line(&self, v: Value) {
        let _ = self.tx.send(v.to_string());
    }

    fn stamp(&mut self, radar: &str) -> u64 {
        let n = self.seq.entry(radar.to_string()).or_default();
        *n += 1;
        *n
    }

    /// Emit a line to radar (stamped); keep it in the history when persisted.
    fn emit(&mut self, radar: &str, mut v: Value) {
        let seq = self.stamp(radar);
        self.emit_with_seq(radar, &mut v, seq);
    }

    fn emit_with_seq(&mut self, radar: &str, v: &mut Value, seq: u64) {
        if v["durability"] == "persisted" && v["provider_event_id"].is_string() {
            self.shared.lock().unwrap().sessions.get_mut(radar).unwrap().events.push(v.clone());
        }
        v["seq"] = json!(seq);
        self.say_line(v.clone());
    }

    fn reply(&self, req: &Value, result: Value) {
        self.say_line(json!({"type":"reply","id":req["id"],"ok":true,"result":result}));
    }

    fn fail(&self, req: &Value, why: &str) {
        self.say_line(json!({"type":"reply","id":req["id"],"ok":false,"error":why}));
    }

    fn pid(&self, radar: &str) -> String {
        format!("provider-{radar}")
    }

    fn id(&self, radar: &str, prefix: &str) -> String {
        let mut p = self.shared.lock().unwrap();
        let h = p.sessions.get_mut(radar).unwrap();
        h.next += 1;
        format!("{prefix}{}", h.next)
    }

    fn say_text(&mut self, radar: &str, text: &str) {
        let pid = self.pid(radar);
        let m = self.id(radar, "msg");
        let (a, b) = text.split_at(text.len() / 2);
        for part in [a, b] {
            self.emit(radar, line(radar, &pid, "ephemeral", "text_delta", Some(&self.id(radar, "d")), json!({"message_id":m,"text":part})));
        }
        let id = self.id(radar, "a");
        self.emit(radar, line(radar, &pid, "persisted", "assistant_message", Some(&id), json!({"message_id":m,"text":text})));
    }

    fn end_turn(&mut self, radar: &str, aborted: bool) {
        let pid = self.pid(radar);
        let id = self.id(radar, "end");
        let payload = if aborted {
            json!({"subtype":"error_during_execution","is_error":true,"errors":["interrupted"]})
        } else {
            json!({"subtype":"success","is_error":false,"result":"","usage":{"input_tokens":1,"output_tokens":2}})
        };
        self.emit(radar, line(radar, &pid, "persisted", "turn_end", Some(&id), payload));
        self.shared.lock().unwrap().sessions.get_mut(radar).unwrap().running = false;
    }

    fn released(&mut self) {
        let hanging: Vec<String> = {
            let p = self.shared.lock().unwrap();
            p.sessions.iter().filter(|(_, h)| h.running).map(|(g, _)| g.clone()).collect()
        };
        for radar in hanging {
            self.say_text(&radar, "Finished");
            self.end_turn(&radar, false);
        }
    }

    fn ask(&mut self, radar: &str, redeliver: bool) {
        let pid = self.pid(radar);
        let tool = self.id(radar, "t");
        self.emit(radar, line(radar, &pid, "persisted", "tool_call", Some(&format!("tool:{tool}")), json!({"tool_use_id":tool,"name":"Bash","input":{"command":"ls -la"}})));
        let req = line(radar, &pid, "persisted", "permission_request", Some("perm:r1"), json!({"request_id":"r1","tool_use_id":tool,"kind":"shell","tool_name":"Bash","command":"ls -la"}));
        self.emit(radar, req.clone());
        if redeliver {
            self.emit(radar, req);
        }
        let mut p = self.shared.lock().unwrap();
        let h = p.sessions.get_mut(radar).unwrap();
        h.pending = Some("r1".into());
        h.running = true;
        h.answered.clear();
        // The tool id is what the result refers to.
        h.events.push(json!({"_tool": tool}));
    }

    fn tool_id(&self, radar: &str) -> String {
        let p = self.shared.lock().unwrap();
        p.sessions[radar].events.iter().rev().find_map(|e| e["_tool"].as_str().map(str::to_string)).unwrap_or_default()
    }

    fn handle(&mut self, req: Value) {
        let op = req["op"].as_str().unwrap_or_default().to_string();
        let radar = req["session"].as_str().unwrap_or_default().to_string();
        self.shared.lock().unwrap().requests.push(req.clone());
        match op.as_str() {
            "catalog" => self.reply(
                &req,
                json!({
                    "agents": [{"name":"scratch-reviewer","description":"Reviews","source":"project"},{"name":"Explore","description":null,"source":null}],
                    "skills": [{"name":"scratch-notes","description":"Notes","source":"project"},{"description":"nameless"}]
                }),
            ),
            "hello" => self.reply(&req, json!({"protocol":1})),
            "status" => self.reply(&req, json!({"installed":true,"path":"/x/claude","version":"2.1.296","account":{"logged_in":true,"auth_method":"claude.ai","email":"user@example.com","subscription_type":"max"}})),
            "models" => self.reply(&req, json!([{"id":"opus","name":"Opus","description":""}])),
            "list" => self.reply(&req, json!([])),
            "delete" => self.reply(&req, json!({})),
            "pending" => self.reply(&req, json!({"count":0})),
            "create" => {
                self.shared.lock().unwrap().sessions.entry(radar.clone()).or_default();
                self.seq.insert(radar.clone(), 0);
                self.reply(&req, json!({"provider_session_id": self.pid(&radar)}));
                // Claude Code says what the session accepts once it has started it.
                self.say_line(json!({"type":"commands","session":radar,"commands":[
                    {"name":"compact","description":"Free up context","argument_hint":"[instructions]"},
                    {"name":"context","description":"Show context usage"},
                ]}));
            }
            "history" => {
                let events: Vec<Value> = {
                    let p = self.shared.lock().unwrap();
                    p.sessions.get(&radar).map(|h| h.events.iter().filter(|e| e["_tool"].is_null()).cloned().collect()).unwrap_or_default()
                };
                let events: Vec<Value> = events
                    .into_iter()
                    .map(|mut e| {
                        e["seq"] = json!(self.stamp(&radar));
                        e
                    })
                    .collect();
                self.reply(&req, json!({"events": events}));
            }
            "send" => {
                if !self.seq.contains_key(&radar) {
                    return self.fail(&req, "unknown session");
                }
                self.reply(&req, json!({}));
                self.send(&radar, req["text"].as_str().unwrap_or_default());
            }
            "answer" => {
                let id = req["request_id"].as_str().unwrap_or_default().to_string();
                let outcome = {
                    let mut p = self.shared.lock().unwrap();
                    let h = p.sessions.get_mut(&radar).unwrap();
                    if h.pending.as_deref() == Some(&id) {
                        h.pending = None;
                        h.answered.insert(id.clone());
                        "answered"
                    } else if h.answered.contains(&id) {
                        "duplicate"
                    } else {
                        "unknown"
                    }
                };
                if outcome == "unknown" {
                    return self.fail(&req, "unknown permission request");
                }
                self.reply(&req, json!({"outcome": outcome}));
                if outcome == "answered" {
                    self.answered(&radar, req["behavior"] == "allow");
                }
            }
            "answer_question" => {
                let id = req["request_id"].as_str().unwrap_or_default().to_string();
                let outcome = {
                    let mut p = self.shared.lock().unwrap();
                    let h = p.sessions.get_mut(&radar).unwrap();
                    if h.pending.as_deref() == Some(&id) {
                        h.pending = None;
                        h.answered.insert(id.clone());
                        "answered"
                    } else if h.answered.contains(&id) {
                        "duplicate"
                    } else {
                        "unknown"
                    }
                };
                if outcome == "unknown" {
                    return self.fail(&req, "unknown question");
                }
                self.reply(&req, json!({"outcome": outcome}));
                if outcome == "answered" {
                    let pid = self.pid(&radar);
                    let answers = req["answers"].clone();
                    self.emit(&radar, line(&radar, &pid, "persisted", "question_resolved", Some("ask-done:q1"), json!({"request_id": id, "answers": answers})));
                    let said = answers.get("Which colour?").and_then(Value::as_str).unwrap_or("nothing").to_string();
                    self.say_text(&radar, &format!("You chose {said}"));
                    self.end_turn(&radar, false);
                }
            }
            "interrupt" => {
                self.reply(&req, json!({}));
                let running = self.shared.lock().unwrap().sessions[&radar].running;
                if running {
                    self.end_turn(&radar, true);
                }
            }
            "close" => self.reply(&req, json!({})),
            other => self.fail(&req, &format!("unknown op {other}")),
        }
    }

    fn send(&mut self, radar: &str, text: &str) {
        let pid = self.pid(radar);
        let (scenario, turn) = {
            let mut p = self.shared.lock().unwrap();
            let sc = p.scenario;
            let h = p.sessions.get_mut(radar).unwrap();
            h.turns += 1;
            (sc, h.turns)
        };
        self.emit(radar, line(radar, &pid, "ephemeral", "turn_started", None, json!({})));
        let uid = self.id(radar, "u");
        self.emit(radar, line(radar, &pid, "persisted", "user_message", Some(&uid), json!({"text":text})));
        match scenario {
            Scenario::Text => {
                self.say_text(radar, "Hello from the fake sidecar");
                self.end_turn(radar, false);
            }
            Scenario::TwoTurns => {
                self.say_text(radar, if turn == 1 { "First answer" } else { "Second answer" });
                self.end_turn(radar, false);
            }
            Scenario::Tool => self.ask(radar, false),
            Scenario::Redelivery => {
                self.say_text(radar, "Once");
                // The last persisted line again, with the same id.
                let last = self.shared.lock().unwrap().sessions[radar].events.last().cloned().unwrap();
                self.emit(radar, last);
                self.ask(radar, true);
            }
            Scenario::Question => {
                // AskUserQuestion, as the sidecar reports it (src/map.js questionEvent).
                self.emit(
                    radar,
                    line(radar, &pid, "persisted", "question_request", Some("ask:q1"), json!({"request_id":"q1","tool_use_id":"tu-ask","questions":[
                        {"question":"Which colour?","header":"Colour","options":[{"label":"red","description":"warm"},{"label":"blue"}],"multi_select":false}
                    ]})),
                );
                let mut p = self.shared.lock().unwrap();
                let h = p.sessions.get_mut(radar).unwrap();
                h.pending = Some("q1".into());
                h.running = true;
                h.answered.clear();
            }
            Scenario::LongTurn => {
                self.say_text(radar, "Working on it");
                self.shared.lock().unwrap().sessions.get_mut(radar).unwrap().running = true;
            }
            Scenario::Reordered => {
                // Two saved messages whose sequence numbers say first, second; delivered second, first.
                let (s1, s2) = (self.stamp(radar), self.stamp(radar));
                let mut first = line(radar, &pid, "persisted", "assistant_message", Some("a-late"), json!({"message_id":"m1","text":"Late first"}));
                let mut second = line(radar, &pid, "persisted", "assistant_message", Some("a-then"), json!({"message_id":"m2","text":"Then second"}));
                {
                    // History keeps provider order.
                    let mut p = self.shared.lock().unwrap();
                    let h = p.sessions.get_mut(radar).unwrap();
                    h.events.push(first.clone());
                    h.events.push(second.clone());
                }
                second["seq"] = json!(s2);
                self.say_line(second);
                first["seq"] = json!(s1);
                self.say_line(first);
                self.end_turn(radar, false);
            }
        }
    }

    fn answered(&mut self, radar: &str, allow: bool) {
        let pid = self.pid(radar);
        let tool = self.tool_id(radar);
        self.emit(radar, line(radar, &pid, "persisted", "permission_resolved", Some("perm-done:r1"), json!({"request_id":"r1","behavior": if allow {"allow"} else {"deny"}})));
        self.emit(
            radar,
            line(radar, &pid, "persisted", "tool_result", Some(&format!("result:{tool}")), json!({"tool_use_id":tool,"is_error":!allow,"content": if allow {"ok"} else {"not now"}})),
        );
        let scenario = self.shared.lock().unwrap().scenario;
        let text = match (scenario, allow) {
            (Scenario::Redelivery, _) => "Twice delivered",
            (_, true) => "Done with the command",
            (_, false) => "Understood",
        };
        self.say_text(radar, text);
        self.end_turn(radar, false);
    }
}
