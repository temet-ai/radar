//! What radar holds is the reference: a refresh asks Slack only for what is
//! newer than it, a start draws from it, and a cache saved before those
//! points existed is read whole once.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use radar_core::Plugin;
use radar_core::plugin::Effect;
use serde_json::{Value, json};

use crate::SlackPlugin;
use crate::client::{ChannelInfo, Fut, Reply, Request, Transport};
use crate::demo::{ALICE, BOB, NOW, TEAM, sample};
use crate::model::{ChannelData, merge_messages};

/// Answers each method from a queue, else with an empty answer, and keeps
/// every request.
#[derive(Default)]
struct Slack {
    history: Mutex<VecDeque<Vec<Value>>>,
    replies: Mutex<VecDeque<Vec<Value>>>,
    seen: Mutex<Vec<Request>>,
}

impl Slack {
    fn new(history: Vec<Vec<Value>>, replies: Vec<Vec<Value>>) -> Arc<Self> {
        Arc::new(Self {
            history: Mutex::new(history.into()),
            replies: Mutex::new(replies.into()),
            seen: Mutex::default(),
        })
    }

    fn asked(&self, method: &str) -> Vec<Request> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }

    fn methods(&self) -> Vec<&'static str> {
        self.seen.lock().unwrap().iter().map(|r| r.method).collect()
    }
}

impl Transport for Slack {
    fn send<'a>(&'a self, req: &'a Request) -> Fut<'a, anyhow::Result<Reply>> {
        self.seen.lock().unwrap().push(req.clone());
        let messages = |q: &Mutex<VecDeque<Vec<Value>>>| q.lock().unwrap().pop_front().unwrap_or_default();
        let body = match req.method {
            "auth.test" => json!({"ok": true, "team_id": "T000ACME1", "user_id": ALICE,
                "url": "https://acme.slack.com/", "team": "Acme", "user": "alice"}),
            "users.info" => json!({"ok": true, "user": {"id": ALICE, "tz_offset": 0}}),
            "conversations.history" => json!({"ok": true, "messages": messages(&self.history)}),
            "conversations.replies" => json!({"ok": true, "messages": messages(&self.replies)}),
            "search.messages" => json!({"ok": true, "messages": {"matches": [], "paging": {"pages": 1}}}),
            "usergroups.list" => json!({"ok": true, "usergroups": []}),
            other => json!({"ok": false, "error": format!("not scripted: {other}")}),
        };
        Box::pin(async move {
            Ok(Reply {
                status: 200,
                body: body.to_string(),
                ..Default::default()
            })
        })
    }
}

fn param<'a>(req: &'a Request, key: &str) -> Option<&'a str> {
    req.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn ts(secs_after_now: i64) -> String {
    format!("{}.000100", NOW + secs_after_now)
}

fn message(at: &str, text: &str) -> Value {
    json!({"ts": at, "user": BOB, "text": text})
}

/// A thread with one reply: `reply` is when its newest reply came.
fn parent(at: &str, replies: u32, reply: &str) -> Value {
    json!({"ts": at, "user": BOB, "text": "a thread", "reply_count": replies,
        "reply_users": [ALICE], "latest_reply": reply})
}

fn window(now: i64) -> String {
    format!("{}.000000", now - 7 * 86_400)
}

/// Run every task the effects start, and the tasks those start, to the end.
fn settle(p: &mut SlackPlugin, mut fx: Vec<Effect>) {
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

/// One watched channel, signed in, its groups just read; read in full at
/// NOW: an older message and a thread with one reply.
fn read_once() -> SlackPlugin {
    let slack = Slack::new(
        vec![vec![parent(&ts(-2000), 1, &ts(-1900)), message(&ts(-3000), "older")]],
        vec![vec![parent(&ts(-2000), 1, &ts(-1900)), message(&ts(-1900), "first reply")]],
    );
    let mut p = sample();
    p.via = crate::Via(Some(slack.clone()));
    p.cfg.channels.truncate(1);
    p.data.channels.clear();
    p.data.channels.insert(
        TEAM.into(),
        ChannelData {
            info: Some(ChannelInfo {
                id: TEAM.into(),
                name: "team".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    p.data.for_you.clear();
    p.data.asked.clear();
    p.data.groups_at = NOW;
    p.now = NOW;
    let fx = p.read();
    settle(&mut p, fx);
    let history = slack.asked("conversations.history");
    assert_eq!(history.len(), 1);
    assert_eq!(param(&history[0], "oldest"), Some(window(NOW).as_str()), "a first read takes the whole window");
    assert_eq!(slack.asked("conversations.replies").len(), 1);
    p
}

/// radar started again at `now` from what `before` saved.
fn restart(before: &SlackPlugin, saved: Value, now: i64, slack: Arc<Slack>) -> SlackPlugin {
    let mut p = SlackPlugin::new();
    p.env = Some(vec![]);
    p.app_running = Some(false);
    p.configure(before.config().as_ref()).unwrap();
    p.restore(saved);
    p.via = crate::Via(Some(slack));
    p.now = now;
    let fx = p.read();
    settle(&mut p, fx);
    p
}

fn held(p: &SlackPlugin) -> Vec<(String, String)> {
    p.data.channels[TEAM]
        .messages
        .iter()
        .map(|m| (m.ts.clone(), m.text.clone()))
        .collect()
}

#[test]
fn a_refresh_after_a_cached_start_asks_only_for_what_is_newer_than_the_newest_held() {
    let before = read_once();
    let slack = Slack::new(vec![vec![message(&ts(60), "new since")]], vec![]);
    let p = restart(&before, before.snapshot().unwrap(), NOW + 120, slack.clone());
    let history = slack.asked("conversations.history");
    assert_eq!(history.len(), 1, "{:?}", slack.methods());
    assert_eq!(param(&history[0], "oldest"), Some(ts(-2000).as_str()), "from the newest message held");
    assert_eq!(param(&history[0], "inclusive"), Some("false"));
    assert!(slack.asked("conversations.replies").is_empty(), "no thread moved");
    assert_eq!(
        held(&p),
        [
            (ts(-3000), "older".to_string()),
            (ts(-2000), "a thread".to_string()),
            (ts(60), "new since".to_string()),
        ],
        "what was held, and the new message after it"
    );
    assert_eq!(p.data.channels[TEAM].replies[&ts(-2000)].len(), 1, "its replies are kept");
}

#[test]
fn a_start_within_a_refresh_of_the_last_read_asks_slack_for_nothing_held() {
    let before = read_once();
    let slack = Slack::new(vec![], vec![]);
    let p = restart(&before, before.snapshot().unwrap(), NOW + 30, slack.clone());
    assert_eq!(
        slack.methods(),
        ["auth.test", "users.info"],
        "signing in only: the channel, For you and the groups were read less than a refresh ago"
    );
    assert_eq!(held(&p), held(&before));
    assert_eq!(p.sync.last_ok, Some(NOW), "as fresh as the read it holds");
    assert_eq!(p.sync.reading, 0);
}

#[test]
fn an_unchanged_channel_costs_one_small_history_call_and_no_thread_call() {
    let mut p = read_once();
    let slack = Slack::new(vec![vec![]], vec![]);
    p.via = crate::Via(Some(slack.clone()));
    p.now = NOW + 120;
    let before = p.data.clone();
    let fx = p.read();
    settle(&mut p, fx);
    let history = slack.asked("conversations.history");
    assert_eq!(history.len(), 1);
    assert_eq!(param(&history[0], "oldest"), Some(ts(-2000).as_str()));
    assert!(slack.asked("conversations.replies").is_empty());
    assert_eq!(p.data.channels[TEAM].messages, before.channels[TEAM].messages);
    assert_eq!(p.data.channels[TEAM].replies, before.channels[TEAM].replies);
}

#[test]
fn every_few_refreshes_and_on_r_the_whole_window_is_read_again() {
    // A whole read, five refreshes after the last one.
    let mut p = read_once();
    let slack = Slack::new(vec![vec![message(&ts(500), "kept")]], vec![]);
    p.via = crate::Via(Some(slack.clone()));
    p.now = NOW + 5 * 120;
    let fx = p.read();
    settle(&mut p, fx);
    let history = slack.asked("conversations.history");
    assert_eq!(param(&history[0], "oldest"), Some(window(NOW + 600).as_str()));
    assert_eq!(held(&p), [(ts(500), "kept".to_string())], "a whole read is what Slack has");
    assert_eq!(p.data.channels[TEAM].swept_at, NOW + 600);

    // `r`, a minute later.
    let slack = Slack::new(vec![vec![message(&ts(500), "kept")]], vec![]);
    p.via = crate::Via(Some(slack.clone()));
    p.now = NOW + 660;
    let fx = p.fetch_now();
    settle(&mut p, fx);
    let history = slack.asked("conversations.history");
    assert_eq!(param(&history[0], "oldest"), Some(window(NOW + 660).as_str()));
    assert!(!p.sync.full_now, "once");
}

#[test]
fn a_thread_with_a_new_reply_is_asked_only_from_its_newest_held_reply() {
    let mut p = read_once();
    // The whole read finds the thread's newest reply moved.
    let slack = Slack::new(
        vec![vec![parent(&ts(-2000), 2, &ts(400)), message(&ts(-3000), "older")]],
        vec![vec![parent(&ts(-2000), 2, &ts(400)), message(&ts(400), "second reply")]],
    );
    p.via = crate::Via(Some(slack.clone()));
    p.now = NOW + 600;
    let fx = p.read();
    settle(&mut p, fx);
    let replies = slack.asked("conversations.replies");
    assert_eq!(replies.len(), 1);
    assert_eq!(param(&replies[0], "ts"), Some(ts(-2000).as_str()));
    assert_eq!(param(&replies[0], "oldest"), Some(ts(-1900).as_str()));
    assert_eq!(param(&replies[0], "inclusive"), Some("false"));
    let got: Vec<_> = p.data.channels[TEAM].replies[&ts(-2000)]
        .iter()
        .map(|m| m.text.as_str())
        .collect();
    assert_eq!(got, ["first reply", "second reply"]);
}

#[test]
fn merging_keeps_the_order_and_one_copy_of_each_message_the_newest_read() {
    let m = |at: &str, text: &str| serde_json::from_value(message(at, text)).unwrap();
    let mut held = vec![m("100.000001", "a"), m("200.000001", "b")];
    merge_messages(
        &mut held,
        vec![m("300.000001", "c"), m("200.000001", "b, edited"), m("150.000001", "late")],
    );
    let got: Vec<_> = held.iter().map(|m| (m.ts.as_str(), m.text.as_str())).collect();
    assert_eq!(
        got,
        [
            ("100.000001", "a"),
            ("150.000001", "late"),
            ("200.000001", "b, edited"),
            ("300.000001", "c"),
        ]
    );
}

#[test]
fn a_cache_saved_before_read_points_were_kept_is_read_whole_once_then_from_its_newest() {
    let before = read_once();
    let mut saved = before.snapshot().unwrap();
    let data = &mut saved["accounts"][0]["data"];
    for key in ["groups_at", "for_you_at", "for_you_swept_at"] {
        data.as_object_mut().unwrap().remove(key);
    }
    for key in ["read_at", "swept_at"] {
        data["channels"][TEAM].as_object_mut().unwrap().remove(key);
    }
    let slack = Slack::new(vec![vec![parent(&ts(-2000), 1, &ts(-1900)), message(&ts(-3000), "older")]], vec![]);
    let mut p = restart(&before, saved, NOW + 30, slack.clone());
    assert!(!p.has_quarantined_cache(), "an older cache is still this account's");
    let history = slack.asked("conversations.history");
    assert_eq!(history.len(), 1);
    assert_eq!(param(&history[0], "oldest"), Some(window(NOW + 30).as_str()), "whole, once");
    assert_eq!(held(&p).len(), 2);

    let slack = Slack::new(vec![vec![]], vec![]);
    p.via = crate::Via(Some(slack.clone()));
    p.now = NOW + 150;
    let fx = p.read();
    settle(&mut p, fx);
    let history = slack.asked("conversations.history");
    assert_eq!(param(&history[0], "oldest"), Some(ts(-2000).as_str()), "then only what is new");
}
