//! A first read with nothing held asks Slack for many names and many
//! threads: those go out a few at a time, once each, rather than one after
//! another (a cold start took 17 s).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use radar_core::Plugin;
use radar_core::plugin::Effect;
use serde_json::{Value, json};

use crate::SlackPlugin;
use crate::client::{Fut, IN_FLIGHT, Reply, Request, Transport};
use crate::demo::{ALICE, BOB, NOW, SVC, TEAM, sample};
use crate::model::Asked;

/// Answers every method from what it was given, slowly for the two that are
/// asked many times, and counts how many of each are in flight.
#[derive(Default)]
struct Slack {
    history: Mutex<Vec<(String, Vec<Value>)>>,
    threads: Mutex<Vec<Value>>,
    seen: Mutex<Vec<Request>>,
    active: AtomicUsize,
    most_names: AtomicUsize,
    most_replies: AtomicUsize,
}

impl Slack {
    fn asked(&self, method: &str) -> Vec<Request> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }
}

fn param<'a>(req: &'a Request, key: &str) -> &'a str {
    req.params
        .iter()
        .find(|(k, _)| k == key)
        .map_or("", |(_, v)| v.as_str())
}

impl Transport for Slack {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, anyhow::Result<Reply>> {
        self.seen.lock().unwrap().push(req.clone());
        let body = match req.method {
            "auth.test" => json!({"ok": true, "team_id": "T000ACME1", "user_id": ALICE,
                "url": "https://acme.slack.com/", "team": "Acme", "user": "alice"}),
            "conversations.info" => {
                let id = param(req, "channel");
                json!({"ok": true, "channel": {"id": id, "name": "team", "is_channel": true}})
            }
            "users.info" => {
                let id = param(req, "user");
                json!({"ok": true, "user": {"id": id, "name": id.to_lowercase(),
                    "profile": {"display_name": format!("name of {id}")}}})
            }
            "conversations.history" => {
                let channel = param(req, "channel");
                let rows = self
                    .history
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(c, _)| c == channel)
                    .map(|(_, rows)| rows.clone())
                    .unwrap_or_default();
                json!({"ok": true, "messages": rows})
            }
            "conversations.replies" => {
                let ts = param(req, "ts");
                let parent = self
                    .threads
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|m| m["ts"] == ts)
                    .cloned()
                    .unwrap_or(json!({"ts": ts}));
                let reply = json!({"ts": format!("{ts}1"), "user": BOB, "text": "a reply",
                    "thread_ts": ts});
                json!({"ok": true, "messages": [parent, reply]})
            }
            "search.messages" => json!({"ok": true, "messages": {"matches": [], "paging": {"pages": 1}}}),
            "usergroups.list" => json!({"ok": true, "usergroups": []}),
            other => json!({"ok": false, "error": format!("not scripted: {other}")}),
        };
        let slow = matches!(req.method, "users.info" | "conversations.replies");
        let most = match req.method {
            "users.info" => Some(&self.most_names),
            "conversations.replies" => Some(&self.most_replies),
            _ => None,
        };
        Box::pin(async move {
            if let (true, Some(most)) = (slow, most) {
                let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                }
                self.active.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(Reply {
                status: 200,
                body: body.to_string(),
                ..Default::default()
            })
        })
    }
}

fn run(p: &mut SlackPlugin, mut fx: Vec<Effect>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    loop {
        let tasks: Vec<_> = fx
            .drain(..)
            .filter_map(|e| match e {
                Effect::Spawn(t) | Effect::SpawnAction { task: t, .. } => Some(t),
                _ => None,
            })
            .collect();
        if tasks.is_empty() {
            return;
        }
        for t in tasks {
            fx.extend(p.on_msg(rt.block_on(t)));
        }
    }
}

fn user(i: usize) -> String {
    format!("UQ{i:07}")
}

fn ts(i: i64) -> String {
    format!("{}.0001{i:02}", NOW - 5000 + i)
}

/// A plugin that holds nothing of its channels and has read its groups.
fn cold(slack: &Arc<Slack>, channels: usize) -> SlackPlugin {
    let mut p = sample();
    p.via = crate::Via(Some(slack.clone()));
    p.cfg.channels.truncate(channels);
    p.data.channels.clear();
    p.data.for_you.clear();
    p.data.asked.clear();
    p.data.groups_at = NOW;
    p.sync.groups_at = NOW;
    p.now = NOW;
    p
}

#[test]
fn the_names_of_a_channels_authors_are_asked_a_few_at_a_time() {
    let slack = Arc::new(Slack::default());
    let rows: Vec<Value> = (0..10)
        .map(|i| json!({"ts": ts(i), "user": user(i as usize), "text": "hello"}))
        .collect();
    slack.history.lock().unwrap().push((TEAM.into(), rows));
    let mut p = cold(&slack, 1);
    let fx = p.read();
    run(&mut p, fx);
    assert_eq!(slack.asked("users.info").len(), 10, "everyone once");
    assert_eq!(slack.most_names.load(Ordering::SeqCst), IN_FLIGHT);
    for i in 0..10 {
        assert_eq!(p.data.users.get(&user(i)), Some(&format!("name of {}", user(i))));
    }
}

#[test]
fn a_person_two_channels_both_name_is_asked_for_once() {
    let slack = Arc::new(Slack::default());
    for channel in [TEAM, SVC] {
        let rows = vec![json!({"ts": ts(1), "user": user(1), "text": "hello"})];
        slack.history.lock().unwrap().push((channel.into(), rows));
    }
    let mut p = cold(&slack, 2);
    let fx = p.read();
    run(&mut p, fx);
    let asked: Vec<String> = slack
        .asked("users.info")
        .iter()
        .map(|r| param(r, "user").to_string())
        .collect();
    assert_eq!(asked, [user(1)]);
    assert_eq!(p.data.users.get(&user(1)).map(String::as_str), Some("name of UQ0000001"));
}

#[test]
fn a_channels_threads_are_read_a_few_at_a_time_and_all_held() {
    let slack = Arc::new(Slack::default());
    let parents: Vec<Value> = (0..10)
        .map(|i| {
            json!({"ts": ts(i), "user": BOB, "text": "a thread", "reply_count": 1,
                "reply_users": [BOB], "latest_reply": format!("{}1", ts(i))})
        })
        .collect();
    *slack.threads.lock().unwrap() = parents.clone();
    slack.history.lock().unwrap().push((TEAM.into(), parents));
    let mut p = cold(&slack, 1);
    let fx = p.read();
    run(&mut p, fx);
    assert_eq!(slack.asked("conversations.replies").len(), 10);
    assert_eq!(slack.most_replies.load(Ordering::SeqCst), IN_FLIGHT);
    let held: BTreeSet<&String> = p.data.channels[TEAM].replies.keys().collect();
    assert_eq!(held.len(), 10, "every thread's replies are held");
    assert!(p.data.channels[TEAM].replies.values().all(|r| r.len() == 1));
}

#[test]
fn the_review_asks_followed_are_read_a_few_at_a_time_and_each_is_updated() {
    let slack = Arc::new(Slack::default());
    let mut p = cold(&slack, 0);
    p.data.asked = (0..8)
        .map(|i| Asked {
            channel: TEAM.into(),
            ts: format!("{}.000000", NOW - 3600 - i),
            ..Default::default()
        })
        .collect();
    let fx = p.read();
    run(&mut p, fx);
    assert_eq!(slack.asked("conversations.replies").len(), 8);
    assert_eq!(slack.most_replies.load(Ordering::SeqCst), IN_FLIGHT);
    assert!(p.data.asked.iter().all(|a| a.replies == 1), "{:?}", p.data.asked);
}
