//! Recorded transport coverage of followed-review reads through the real task
//! and result path. Unrelated For-you tasks are dropped without being polled.

use std::collections::BTreeSet;
use std::sync::Arc;

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome, Reason, SubjectRole};
use radar_core::plugin::{Effect, Task};
use serde_json::{Value, json};

use crate::client::{MessagesRead, Reaction, Recorded, Reply};
use crate::demo::{BOB, NOW, SVC, TEAM, sample};
use crate::model::Asked;
use crate::read::Read;
use crate::sync_history::{ChannelRead, NamesEvidence};
use crate::{SlackMsg, SlackPlugin, SyncErrorKey};

const TARGET: &str = "slack.sync.followed.thread";
const MESSAGES: &str = "slack.sync.followed.messages";
const ATTEMPTED: &str = "slack.sync.followed.targets.attempted";
const PARENTS: &str = "slack.sync.followed.parents.observed";
const ERRORS: &str = "slack.sync.followed.targets.errors";
const MISSING: &str = "slack.sync.followed.parents.missing";
const TRUNCATED: &str = "slack.sync.lists.truncated";
const INCOMPLETE: &str = "slack.sync.pages.incomplete";

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn page(messages: Vec<Value>) -> anyhow::Result<Reply> {
    ok(json!({ "ok": true, "messages": messages }))
}

fn fail() -> anyhow::Result<Reply> {
    ok(json!({ "ok": false, "error": "private_error_canary" }))
}

fn ask(i: i64) -> Asked {
    Asked {
        channel: TEAM.into(),
        ts: format!("{}.000000", NOW - 1000 + i),
        mrs: vec![(
            "private_mr_canary".into(),
            "https://private.example/review".into(),
        )],
        replies: 71,
        reactions: vec![Reaction {
            name: "private_cached_reaction_canary".into(),
            count: 3,
            users: vec![BOB.into()],
        }],
        ..Default::default()
    }
}

fn parent(a: &Asked, replies: u32) -> Value {
    json!({
        "ts": a.ts,
        "user": BOB,
        "text": "private_parent_body_canary",
        "reply_count": replies,
        "reactions": [{
            "name": "private_new_reaction_canary", "count": 4, "users": [BOB]
        }],
    })
}

fn reply(i: usize) -> Value {
    json!({
        "ts": format!("{}.000000", NOW - 500 + i as i64),
        "user": BOB,
        "text": "private_reply_body_canary",
    })
}

fn recorded(
    asked: Vec<Asked>,
    replies: Vec<anyhow::Result<Reply>>,
) -> (SlackPlugin, Arc<Recorded>) {
    let rec = Recorded::new(replies);
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    p.cfg.channels.clear();
    p.data.asked = asked;
    (p, rec)
}

fn take_task(fx: Vec<Effect>) -> (ActionDescriptor, Task) {
    let mut tasks = fx.into_iter().filter_map(|effect| match effect {
        Effect::SpawnAction { action, task } if action.kind.as_str() == "slack.sync.followed" => {
            Some((action, task))
        }
        _ => None,
    });
    let (action, task) = tasks.next().expect("one followed-review task");
    assert_eq!(action.kind.as_str(), "slack.sync.followed");
    assert!(
        tasks.next().is_none(),
        "all selected targets share one task"
    );
    (action, task)
}

fn run(p: &mut SlackPlugin, task: Task) -> Vec<Effect> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    p.on_msg(rt.block_on(task))
}

fn read(p: &mut SlackPlugin) -> Vec<Effect> {
    let (_, task) = take_task(p.read());
    run(p, task)
}

/// Complete the unrelated task without polling its transport future.
fn for_you(p: &mut SlackPlugin, result: Result<(), &str>) -> Vec<Effect> {
    p.on_msg(Box::new(SlackMsg::Read {
        stamp: p.read_stamp(),
        read: Box::new(Read::ForYou {
            groups: None,
            result: result.map(|()| vec![]).map_err(str::to_owned),
            names: vec![],
            action: super::for_you_action("T000ACME1"),
            evidence: Default::default(),
        }),
    }))
}

fn channel(p: &mut SlackPlugin, result: Result<(), &str>) -> Vec<Effect> {
    p.on_msg(Box::new(SlackMsg::Read {
        stamp: p.read_stamp(),
        read: Box::new(Read::Channel {
            id: TEAM.into(),
            result: result
                .map(|()| ChannelRead {
                    info: None,
                    list: MessagesRead {
                        messages: vec![],
                        evidence: Default::default(),
                    },
                })
                .map_err(str::to_owned),
            names: vec![],
            names_evidence: NamesEvidence::default(),
            action: super::action("slack.sync.channel", "T000ACME1", TEAM),
        }),
    }))
}

fn followed_error(p: &SlackPlugin, outcome: Outcome) {
    let expected = match outcome {
        Outcome::Failed => "followed review threads could not be refreshed",
        Outcome::Partial => "some followed review threads could not be fully refreshed",
        _ => panic!("expected a failed or partial followed read"),
    };
    assert_eq!(
        p.sync
            .errors
            .get(&SyncErrorKey::Followed)
            .map(String::as_str),
        Some(expected),
        "only a generic explanation enters the transient UI error"
    );
}

fn persists(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::PersistCache))
}

fn observations(fx: &[Effect]) -> Vec<(&ActionDescriptor, Outcome)> {
    fx.iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((action, *outcome)),
            _ => None,
        })
        .collect()
}

fn matching<'a>(fx: &'a [Effect], kind: &str) -> Vec<(&'a ActionDescriptor, Outcome)> {
    observations(fx)
        .into_iter()
        .filter(|(a, _)| a.kind.as_str() == kind)
        .collect()
}

fn report(fx: &[Effect], expected: Outcome) {
    let reports: Vec<_> = fx
        .iter()
        .filter_map(|effect| match effect {
            Effect::Report(outcome) => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(reports, [expected]);
}

fn count(fx: &[Effect], kind: &str, n: u32, outcome: Outcome, truncated: bool) {
    let matching = matching(fx, kind);
    assert_eq!(matching.len(), 1, "{kind}: {fx:?}");
    let (a, got) = matching[0];
    assert_eq!(
        (a.metadata.count, got, a.metadata.truncated),
        (Some(n), outcome, truncated),
        "{kind}"
    );
}

fn absent(fx: &[Effect], kind: &str) {
    assert!(matching(fx, kind).is_empty(), "{kind}: {fx:?}");
}

fn identity(a: &ActionDescriptor, team: &str, kind: &str, id: &str) {
    assert_eq!(a.subjects.len(), 1);
    let subject = &a.subjects[0];
    assert_eq!(subject.key.provider, "slack");
    assert_eq!(subject.key.instance, team);
    assert_eq!(subject.key.kind, kind);
    assert_eq!(subject.key.id, id);
    assert_eq!(subject.role, SubjectRole::Primary);
    assert!(subject.url.is_none());
}

fn target(
    fx: &[Effect],
    i: usize,
    asked: &Asked,
    outcome: Outcome,
    reason: Option<Reason>,
    truncated: bool,
    messages: Option<u32>,
) {
    let (a, got) = matching(fx, TARGET)[i];
    identity(
        a,
        "T000ACME1",
        "thread",
        &format!("{}/{}", asked.channel, asked.ts),
    );
    assert_eq!(got, outcome);
    assert_eq!(a.metadata.reason, reason);
    assert_eq!(a.metadata.truncated, truncated);
    assert_eq!(
        a.metadata.count, messages,
        "target carries only actual returned entries"
    );
}

fn requests(rec: &Recorded) -> Vec<(String, String)> {
    rec.requests()
        .iter()
        .map(|r| {
            assert_eq!(r.method, "conversations.replies");
            assert!(!r.write);
            assert!(r.body.is_none());
            let param = |key| r.params.iter().find(|(k, _)| k == key).unwrap().1.clone();
            assert_eq!(param("limit"), "200");
            (param("channel"), param("ts"))
        })
        .collect()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn empty_followed_selection_spawns_no_scoped_task() {
    for asked in [
        vec![],
        vec![Asked {
            ts: format!("{}.000000", NOW - 14 * 86_400),
            ..ask(0)
        }],
    ] {
        let (mut p, rec) = recorded(asked, vec![]);
        p.sync.errors = [
            (SyncErrorKey::Bootstrap, "workspace unavailable".into()),
            (
                SyncErrorKey::Channel("followed review asks".into()),
                "channel unavailable".into(),
            ),
            (SyncErrorKey::ForYou, "for you unavailable".into()),
            (SyncErrorKey::Followed, "stale followed error".into()),
        ]
        .into_iter()
        .collect();
        let mut unrelated = p.sync.errors.clone();
        unrelated.remove(&SyncErrorKey::Followed);
        let previous = p.sync.last_ok;
        let fx = p.read();
        assert_eq!(p.sync.errors, unrelated);
        assert_eq!(p.sync.last_ok, previous);
        assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "slack.sync.followed")));
        assert!(observations(&fx).is_empty());
        assert!(
            fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "slack.sync.for_you")),
            "For-you remains unpolled"
        );
        drop(fx);
        assert!(rec.requests().is_empty());
        let fx = for_you(&mut p, Ok(()));
        unrelated.remove(&SyncErrorKey::ForYou);
        assert_eq!(p.sync.errors, unrelated);
        assert_eq!(p.sync.last_ok, previous);
        assert_eq!(p.sync.reading, 0);
        assert!(persists(&fx));
        assert!(
            !observations(&fx)
                .iter()
                .any(|(a, _)| a.kind.as_str().starts_with("slack.sync.followed"))
        );
        assert!(rec.requests().is_empty());

        // With no unrelated failure, having nothing left to follow permits a
        // fresh sync once the ordinary For-you task completes.
        p.sync.errors = [(SyncErrorKey::Followed, "stale followed error".into())]
            .into_iter()
            .collect();
        p.now += 60;
        let fx = p.read();
        assert!(p.sync.errors.is_empty());
        assert_eq!(p.sync.last_ok, previous);
        assert_eq!(p.sync.reading, 1);
        assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "slack.sync.followed")));
        assert!(observations(&fx).is_empty());
        drop(fx);
        let fx = for_you(&mut p, Ok(()));
        assert_eq!(p.sync.reading, 0);
        assert_eq!(p.sync.last_ok, Some(NOW + 60));
        assert!(persists(&fx));
        assert!(rec.requests().is_empty());
    }
}

#[test]
fn complete_parent_updates_legacy_cache_but_counts_only_returned_messages() {
    let asked = ask(0);
    let declared = 987_654_321;
    let (mut p, rec) = recorded(
        vec![asked.clone()],
        vec![page(vec![parent(&asked, declared), reply(0)])],
    );
    let (action, task) = take_task(p.read());
    identity(&action, "T000ACME1", "workspace", "workspace");
    let before = p.snapshot().unwrap();
    let fx = run(&mut p, task);
    report(&fx, Outcome::Succeeded);
    count(&fx, ATTEMPTED, 1, Outcome::Succeeded, false);
    count(&fx, PARENTS, 1, Outcome::Succeeded, false);
    count(&fx, MESSAGES, 2, Outcome::Succeeded, false);
    target(&fx, 0, &asked, Outcome::Succeeded, None, false, Some(2));
    assert_eq!(observations(&fx).len(), 4);
    assert_eq!(p.data.asked[0].replies, declared);
    assert_eq!(
        p.data.asked[0].reactions,
        [Reaction {
            name: "private_new_reaction_canary".into(),
            count: 4,
            users: vec![BOB.into()]
        }]
    );
    assert_eq!(p.data.asked[0].mrs, asked.mrs);
    assert_eq!(requests(&rec), [(TEAM.into(), asked.ts)]);
    assert!(
        observations(&fx)
            .iter()
            .all(|(a, _)| a.metadata.count != Some(declared))
    );
    let after = p.snapshot().unwrap();
    assert_eq!(keys(&before), keys(&after));
    assert_eq!(
        keys(super::tests::account_data(&before)),
        keys(super::tests::account_data(&after))
    );
    assert_eq!(
        keys(&super::tests::account_data(&before)["asked"][0]),
        keys(&super::tests::account_data(&after)["asked"][0])
    );
    for transient in [
        "evidence",
        "targets",
        "parent_found",
        "incomplete_pages",
        "slack.sync.followed",
    ] {
        assert!(!after.to_string().contains(transient));
    }
}

#[test]
fn fallback_reply_count_excludes_parent_but_message_evidence_includes_it() {
    let asked = ask(0);
    let (mut p, rec) = recorded(
        vec![asked.clone()],
        vec![page(vec![reply(0), parent(&asked, 0), reply(1)])],
    );
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    count(&fx, MESSAGES, 3, Outcome::Succeeded, false);
    target(&fx, 0, &asked, Outcome::Succeeded, None, false, Some(3));
    assert_eq!(p.data.asked[0].replies, 2);
    assert_eq!(
        requests(&rec).len(),
        1,
        "parent is matched by ts, not list position"
    );
}

#[test]
fn mixed_success_error_and_missing_parent_continue_through_every_target() {
    let asked: Vec<_> = (0..4).map(ask).collect();
    let (mut p, rec) = recorded(
        asked.clone(),
        vec![
            page(vec![parent(&asked[0], 6), reply(0)]),
            fail(),
            page(vec![reply(1)]),
            page(vec![parent(&asked[3], 9)]),
        ],
    );
    let previous = p.sync.last_ok;
    let fx = read(&mut p);
    report(&fx, Outcome::Partial);
    count(&fx, ATTEMPTED, 4, Outcome::Succeeded, false);
    count(&fx, PARENTS, 2, Outcome::Succeeded, false);
    count(&fx, ERRORS, 1, Outcome::Failed, false);
    count(&fx, MISSING, 1, Outcome::Failed, false);
    target(&fx, 0, &asked[0], Outcome::Succeeded, None, false, Some(2));
    target(
        &fx,
        1,
        &asked[1],
        Outcome::Failed,
        Some(Reason::Unspecified),
        false,
        None,
    );
    target(
        &fx,
        2,
        &asked[2],
        Outcome::Failed,
        Some(Reason::Unavailable),
        false,
        Some(1),
    );
    target(&fx, 3, &asked[3], Outcome::Succeeded, None, false, Some(1));
    let messages = matching(&fx, MESSAGES);
    assert_eq!(
        messages
            .iter()
            .map(|(a, o)| (a.metadata.count, *o))
            .collect::<Vec<_>>(),
        [
            (Some(2), Outcome::Succeeded),
            (Some(1), Outcome::Succeeded),
            (Some(1), Outcome::Succeeded)
        ]
    );
    assert_eq!(
        messages[1].0.subjects[0].key.id,
        format!("{}/{}", TEAM, asked[2].ts)
    );
    assert_eq!(p.data.asked[0].replies, 6);
    assert_eq!(p.data.asked[1], asked[1]);
    assert_eq!(p.data.asked[2], asked[2]);
    assert_eq!(p.data.asked[3].replies, 9);
    assert_eq!(
        requests(&rec),
        asked
            .iter()
            .map(|a| (a.channel.clone(), a.ts.clone()))
            .collect::<Vec<_>>()
    );
    assert!(!format!("{:?}", observations(&fx)).contains("private_error_canary"));
    followed_error(&p, Outcome::Partial);
    assert_eq!(p.sync.last_ok, previous);
    assert!(!persists(&fx));
    let fx = for_you(&mut p, Ok(()));
    assert_eq!(p.sync.reading, 0);
    assert_eq!(p.sync.last_ok, previous);
    assert!(
        persists(&fx),
        "successful cache updates survive a partial read"
    );
    let cached: Vec<Asked> =
        serde_json::from_value(super::tests::account_data(&p.snapshot().unwrap())["asked"].clone())
            .unwrap();
    assert_eq!(cached, p.data.asked);
    followed_error(&p, Outcome::Partial);
}

#[test]
fn all_read_errors_fail_without_fabricating_empty_successful_lists() {
    let asked = vec![ask(0), ask(1)];
    let (mut p, rec) = recorded(
        asked.clone(),
        vec![
            fail(),
            Err(anyhow::anyhow!("private_transport_error_canary")),
        ],
    );
    let previous = p.sync.last_ok;
    let fx = read(&mut p);
    report(&fx, Outcome::Failed);
    count(&fx, ATTEMPTED, 2, Outcome::Succeeded, false);
    count(&fx, PARENTS, 0, Outcome::Succeeded, false);
    count(&fx, ERRORS, 2, Outcome::Failed, false);
    for (i, a) in asked.iter().enumerate() {
        target(
            &fx,
            i,
            a,
            Outcome::Failed,
            Some(Reason::Unspecified),
            false,
            None,
        );
    }
    for kind in [MESSAGES, MISSING, TRUNCATED, INCOMPLETE] {
        absent(&fx, kind);
    }
    assert_eq!(observations(&fx).len(), 5);
    assert_eq!(p.data.asked, asked);
    assert_eq!(requests(&rec).len(), 2);
    let safe = format!("{:?}", observations(&fx));
    assert!(!safe.contains("private_transport_error_canary"));
    assert!(!safe.contains("private_error_canary"));
    followed_error(&p, Outcome::Failed);
    assert_eq!(p.sync.last_ok, previous);
    assert!(!persists(&fx));
    let fx = for_you(&mut p, Ok(()));
    assert_eq!(p.sync.reading, 0);
    assert_eq!(p.sync.last_ok, previous);
    assert!(persists(&fx));
    followed_error(&p, Outcome::Failed);
}

#[test]
fn failed_followed_history_does_not_advance_last_ok_and_still_persists_cache() {
    for previous in [None, Some(NOW - 120)] {
        let asked = vec![ask(0)];
        let (mut p, rec) = recorded(asked.clone(), vec![fail()]);
        p.sync.last_ok = previous;
        p.sync.errors.clear();
        let fx = p.read();
        assert_eq!(p.sync.reading, 2);
        assert_eq!(
            fx.iter().filter(|e| matches!(e, Effect::Spawn(_))).count(),
            0
        );
        assert_eq!(
            fx.iter()
                .filter(|e| matches!(e, Effect::SpawnAction { .. }))
                .count(),
            2
        );
        let (_, task) = take_task(fx);
        let for_you = for_you(&mut p, Ok(()));
        assert_eq!(p.sync.reading, 1);
        assert_eq!(p.sync.last_ok, previous);
        assert!(!for_you.iter().any(|e| matches!(e, Effect::PersistCache)));
        assert!(
            rec.requests().is_empty(),
            "the unrelated task finishes without network"
        );

        let fx = run(&mut p, task);
        report(&fx, Outcome::Failed);
        followed_error(&p, Outcome::Failed);
        assert_eq!(p.sync.reading, 0);
        assert_eq!(
            p.sync.last_ok, previous,
            "an unsuccessful followed read cannot make the whole sync look fresh"
        );
        assert!(fx.iter().any(|e| matches!(e, Effect::PersistCache)));
        assert_eq!(p.data.asked, asked);
        assert_eq!(requests(&rec).len(), 1);
    }
}

#[test]
fn missing_parent_fails_even_when_the_returned_list_is_complete_truncated_or_incomplete() {
    let cases = [
        (json!({ "ok": true, "messages": [] }), 0, false, false),
        (
            json!({ "ok": true, "messages": [reply(0)] }),
            1,
            false,
            false,
        ),
        (
            json!({ "ok": true, "messages": (0..201).map(reply).collect::<Vec<_>>() }),
            200,
            true,
            false,
        ),
        (
            json!({ "ok": true, "messages": [reply(0)], "has_more": true }),
            1,
            false,
            true,
        ),
        (json!({ "ok": true, "messages": false }), 0, false, true),
    ];
    for (body, n, truncated, incomplete) in cases {
        let asked = vec![ask(0), ask(1)];
        let (mut p, rec) = recorded(asked.clone(), vec![ok(body.clone()), ok(body)]);
        let previous = p.sync.last_ok;
        let fx = read(&mut p);
        report(&fx, Outcome::Failed);
        count(&fx, ATTEMPTED, 2, Outcome::Succeeded, false);
        count(&fx, PARENTS, 0, Outcome::Succeeded, false);
        count(&fx, MISSING, 2, Outcome::Failed, false);
        absent(&fx, ERRORS);
        for (i, a) in asked.iter().enumerate() {
            target(
                &fx,
                i,
                a,
                Outcome::Failed,
                Some(Reason::Unavailable),
                truncated,
                Some(n),
            );
        }
        let list_outcome = if truncated || incomplete {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        let lists = matching(&fx, MESSAGES);
        assert_eq!(lists.len(), 2);
        for (a, outcome) in lists {
            assert_eq!(
                (a.metadata.count, outcome, a.metadata.truncated),
                (Some(n), list_outcome, truncated)
            );
            assert_eq!(
                a.metadata.reason, None,
                "missing root does not taint returned-list evidence"
            );
        }
        assert_eq!(
            matching(&fx, TRUNCATED).len(),
            if truncated { 2 } else { 0 }
        );
        assert_eq!(
            matching(&fx, INCOMPLETE).len(),
            if incomplete { 2 } else { 0 }
        );
        assert_eq!(p.data.asked, asked);
        assert_eq!(requests(&rec).len(), 2);
        followed_error(&p, Outcome::Failed);
        assert_eq!(p.sync.last_ok, previous);
        assert!(!persists(&fx));
        let fx = for_you(&mut p, Ok(()));
        assert_eq!(p.sync.reading, 0);
        assert_eq!(p.sync.last_ok, previous);
        assert!(persists(&fx));
        followed_error(&p, Outcome::Failed);
    }
}

#[test]
fn observed_parent_with_list_gaps_is_partial_and_exact_row_bound_can_be_complete() {
    for (rows, cursor, incomplete) in [
        (200, false, false),
        (201, false, false),
        (200, true, false),
        (2, false, true),
        (201, false, true),
    ] {
        let asked = ask(0);
        let mut messages = vec![parent(&asked, 0)];
        messages.extend((0..rows - 1).map(reply));
        let mut body = json!({ "ok": true, "messages": messages });
        if cursor {
            body["response_metadata"] = json!({ "next_cursor": "private_unread_cursor_canary" });
        }
        if incomplete {
            body["has_more"] = json!(true);
        }
        let (mut p, rec) = recorded(vec![asked.clone()], vec![ok(body)]);
        let previous = p.sync.last_ok;
        let fx = read(&mut p);
        let truncated = rows > 200 || cursor;
        let outcome = if truncated || incomplete {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        report(&fx, outcome);
        target(
            &fx,
            0,
            &asked,
            outcome,
            None,
            truncated,
            Some(rows.min(200) as u32),
        );
        count(&fx, PARENTS, 1, Outcome::Succeeded, false);
        count(&fx, MESSAGES, rows.min(200) as u32, outcome, truncated);
        if truncated {
            count(&fx, TRUNCATED, 1, Outcome::Partial, true);
        } else {
            absent(&fx, TRUNCATED);
        }
        if incomplete {
            count(&fx, INCOMPLETE, 1, Outcome::Partial, false);
        } else {
            absent(&fx, INCOMPLETE);
        }
        assert_eq!(p.data.asked[0].replies, rows.min(200) as u32 - 1);
        assert_eq!(
            requests(&rec).len(),
            1,
            "the existing 200-entry bound stops further requests"
        );
        assert!(!format!("{:?}", observations(&fx)).contains("private_unread_cursor_canary"));
        assert_eq!(p.sync.last_ok, previous, "For-you is still running");
        assert!(!persists(&fx));
        if outcome == Outcome::Partial {
            followed_error(&p, outcome);
        } else {
            assert!(!p.sync.errors.contains_key(&SyncErrorKey::Followed));
        }
        let fx = for_you(&mut p, Ok(()));
        assert_eq!(p.sync.reading, 0);
        assert_eq!(
            p.sync.last_ok,
            if outcome == Outcome::Succeeded {
                Some(NOW)
            } else {
                previous
            }
        );
        assert!(
            persists(&fx),
            "observed parent updates are always persisted"
        );
        assert_eq!(
            super::tests::account_data(&p.snapshot().unwrap())["asked"][0]["replies"],
            json!(rows.min(200) as u32 - 1)
        );
    }
}

#[test]
fn complete_followed_recovery_waits_for_the_last_task_in_either_completion_order() {
    for followed_first in [true, false] {
        let asked = ask(0);
        let (mut p, rec) = recorded(
            vec![asked.clone()],
            vec![fail(), page(vec![parent(&asked, 8)])],
        );
        let previous = p.sync.last_ok;
        report(&read(&mut p), Outcome::Failed);
        assert!(persists(&for_you(&mut p, Ok(()))));
        followed_error(&p, Outcome::Failed);
        assert_eq!(p.sync.last_ok, previous);

        p.now = NOW + 60;
        let (_, task) = take_task(p.read());
        assert_eq!(p.sync.reading, 2);
        followed_error(&p, Outcome::Failed);
        let finish = if followed_first {
            let fx = run(&mut p, task);
            report(&fx, Outcome::Succeeded);
            assert!(p.sync.errors.is_empty());
            assert_eq!(p.sync.reading, 1);
            assert_eq!(p.sync.last_ok, previous);
            assert!(!persists(&fx));
            p.now += 1;
            for_you(&mut p, Ok(()))
        } else {
            let fx = for_you(&mut p, Ok(()));
            followed_error(&p, Outcome::Failed);
            assert_eq!(p.sync.reading, 1);
            assert_eq!(p.sync.last_ok, previous);
            assert!(!persists(&fx));
            p.now += 1;
            let fx = run(&mut p, task);
            report(&fx, Outcome::Succeeded);
            fx
        };
        assert_eq!(p.sync.reading, 0);
        assert_eq!(p.sync.last_ok, Some(NOW + 61));
        assert!(p.sync.errors.is_empty());
        assert!(persists(&finish));
        assert_eq!(p.data.asked[0].replies, 8);
        assert_eq!(requests(&rec), vec![(TEAM.into(), asked.ts); 2]);
    }
}

#[test]
fn complete_followed_recovery_clears_only_its_own_error() {
    let asked = ask(0);
    let (mut p, rec) = recorded(vec![asked.clone()], vec![page(vec![parent(&asked, 7)])]);
    p.sync.errors = [
        (SyncErrorKey::Bootstrap, "workspace unavailable".into()),
        (
            SyncErrorKey::Channel("followed review asks".into()),
            "channel unavailable".into(),
        ),
        (SyncErrorKey::ForYou, "for you unavailable".into()),
        (SyncErrorKey::Followed, "stale followed error".into()),
    ]
    .into_iter()
    .collect();
    let mut unrelated = p.sync.errors.clone();
    unrelated.remove(&SyncErrorKey::Followed);
    let previous = p.sync.last_ok;
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    assert_eq!(p.sync.errors, unrelated);
    assert_eq!(p.sync.last_ok, previous);
    assert!(!persists(&fx));
    let fx = for_you(&mut p, Ok(()));
    unrelated.remove(&SyncErrorKey::ForYou);
    assert_eq!(p.sync.errors, unrelated);
    assert_eq!(p.sync.reading, 0);
    assert_eq!(p.sync.last_ok, previous);
    assert!(persists(&fx));
    assert_eq!(p.data.asked[0].replies, 7);
    assert_eq!(requests(&rec), [(TEAM.into(), asked.ts)]);
    let cache = p.snapshot().unwrap().to_string();
    for transient in [
        "stale followed error",
        "workspace unavailable",
        "channel unavailable",
        "for you unavailable",
    ] {
        assert!(!cache.contains(transient), "sync errors stay transient");
    }
}

#[test]
fn a_channel_named_followed_review_asks_cannot_clear_followed_failures() {
    for channel_first in [true, false] {
        let (mut p, rec) = recorded(vec![ask(0)], vec![fail()]);
        p.cfg.channels = crate::demo::settings().channels;
        p.cfg.channels.truncate(1);
        p.cfg.channels[0].alias = "followed review asks".into();
        let channel_key = SyncErrorKey::Channel("followed review asks".into());
        p.sync
            .errors
            .insert(channel_key.clone(), "old channel error".into());
        let previous = p.sync.last_ok;
        let fx = p.read();
        assert_eq!(p.sync.reading, 3);
        let (_, task) = take_task(
            fx.into_iter()
                .filter(|effect| {
                    matches!(effect, Effect::SpawnAction { action, .. }
                        if action.kind.as_str() == "slack.sync.followed")
                })
                .collect(),
        );
        if channel_first {
            report(&channel(&mut p, Ok(())), Outcome::Succeeded);
            assert!(!p.sync.errors.contains_key(&channel_key));
            report(&run(&mut p, task), Outcome::Failed);
        } else {
            report(&run(&mut p, task), Outcome::Failed);
            followed_error(&p, Outcome::Failed);
            assert!(p.sync.errors.contains_key(&channel_key));
            report(&channel(&mut p, Ok(())), Outcome::Succeeded);
        }
        followed_error(&p, Outcome::Failed);
        assert!(!p.sync.errors.contains_key(&channel_key));
        assert_eq!(p.sync.errors.len(), 1);
        assert_eq!(p.sync.reading, 1);
        assert_eq!(p.sync.last_ok, previous);
        let fx = for_you(&mut p, Ok(()));
        followed_error(&p, Outcome::Failed);
        assert_eq!(p.sync.reading, 0);
        assert_eq!(p.sync.last_ok, previous);
        assert!(persists(&fx));
        assert_eq!(
            requests(&rec).len(),
            1,
            "only followed transport was polled"
        );
    }
}

#[test]
fn a_channel_named_for_you_does_not_share_for_you_errors_in_either_completion_order() {
    for channel_first in [true, false] {
        for channel_succeeds in [true, false] {
            let (mut p, rec) = recorded(vec![], vec![]);
            p.cfg.channels = crate::demo::settings().channels;
            p.cfg.channels.truncate(1);
            p.cfg.channels[0].alias = "for you".into();
            let channel_key = SyncErrorKey::Channel("for you".into());
            p.sync
                .errors
                .insert(channel_key.clone(), "old channel error".into());
            p.sync
                .errors
                .insert(SyncErrorKey::ForYou, "old for you error".into());
            let previous = p.sync.last_ok;
            drop(p.read());
            assert_eq!(p.sync.reading, 2);
            let channel_result = if channel_succeeds {
                Ok(())
            } else {
                Err("channel unavailable")
            };
            let for_you_result = if channel_succeeds {
                Err("for you unavailable")
            } else {
                Ok(())
            };
            let (first, finish) = if channel_first {
                let first = channel(&mut p, channel_result);
                assert_eq!(p.sync.last_ok, previous);
                (first, for_you(&mut p, for_you_result))
            } else {
                let first = for_you(&mut p, for_you_result);
                assert_eq!(p.sync.last_ok, previous);
                (first, channel(&mut p, channel_result))
            };
            let expected = if channel_succeeds {
                (SyncErrorKey::ForYou, "for you unavailable".into())
            } else {
                (channel_key, "channel unavailable".into())
            };
            assert_eq!(p.sync.errors, [expected].into_iter().collect());
            assert_eq!(p.sync.reading, 0);
            assert_eq!(p.sync.last_ok, previous);
            assert!(!persists(&first));
            assert!(persists(&finish));
            assert!(
                rec.requests().is_empty(),
                "unrelated tasks are never polled"
            );
        }
    }
}

#[test]
fn duplicate_asked_entries_keep_separate_results_and_legacy_first_cache_match() {
    let first = ask(0);
    let mut second = first.clone();
    second.mrs = vec![("second_mr".into(), "https://private.example/second".into())];
    second.replies = 82;
    let mut third = first.clone();
    third.replies = 93;
    let (mut p, rec) = recorded(
        vec![first.clone(), second.clone(), third.clone()],
        vec![
            page(vec![parent(&first, 5)]),
            fail(),
            page(vec![parent(&first, 9), reply(0)]),
        ],
    );
    let fx = read(&mut p);
    report(&fx, Outcome::Partial);
    count(&fx, ATTEMPTED, 3, Outcome::Succeeded, false);
    count(&fx, PARENTS, 2, Outcome::Succeeded, false);
    count(&fx, ERRORS, 1, Outcome::Failed, false);
    let targets = matching(&fx, TARGET);
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0].0.subjects, targets[1].0.subjects);
    assert_eq!(targets[1].0.subjects, targets[2].0.subjects);
    target(&fx, 0, &first, Outcome::Succeeded, None, false, Some(1));
    target(
        &fx,
        1,
        &first,
        Outcome::Failed,
        Some(Reason::Unspecified),
        false,
        None,
    );
    target(&fx, 2, &first, Outcome::Succeeded, None, false, Some(2));
    assert_eq!(
        matching(&fx, MESSAGES)
            .iter()
            .map(|(a, _)| a.metadata.count)
            .collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    assert_eq!(
        p.data.asked[0].replies, 9,
        "later successful update still reaches the first cache match"
    );
    assert_eq!(p.data.asked[0].mrs, first.mrs);
    assert_eq!(p.data.asked[1], second);
    assert_eq!(p.data.asked[2], third);
    assert_eq!(requests(&rec), vec![(TEAM.into(), first.ts); 3]);
}

#[test]
fn strict_fourteen_day_selection_keeps_second_precision_order_and_future_timestamps() {
    let boundary = NOW - 14 * 86_400;
    let timestamps = [
        format!("{}.000000", boundary - 1),
        format!("{boundary}.000000"),
        format!("{boundary}.999999"),
        format!("{}.000000", boundary + 1),
        format!("{}.000000", NOW + 86_400),
        format!("{NOW}.000000"),
    ];
    let asked: Vec<_> = timestamps
        .into_iter()
        .enumerate()
        .map(|(i, ts)| Asked {
            ts,
            test: i == 4,
            ..ask(i as i64)
        })
        .collect();
    let selected = &asked[3..];
    let replies = selected.iter().map(|a| page(vec![parent(a, 0)])).collect();
    let (mut p, rec) = recorded(asked.clone(), replies);
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    count(&fx, ATTEMPTED, 3, Outcome::Succeeded, false);
    count(&fx, PARENTS, 3, Outcome::Succeeded, false);
    assert_eq!(
        requests(&rec),
        selected
            .iter()
            .map(|a| (a.channel.clone(), a.ts.clone()))
            .collect::<Vec<_>>()
    );
    for (i, a) in selected.iter().enumerate() {
        target(&fx, i, a, Outcome::Succeeded, None, false, Some(1));
    }
    assert_eq!(
        &p.data.asked[..3],
        &asked[..3],
        "exact-boundary fractions remain excluded by the existing seconds comparison"
    );
    assert!(p.data.asked[3..].iter().all(|a| a.replies == 0));
}

#[test]
fn dispatch_scopes_survive_reconfiguration_and_retain_no_private_display_or_payload_data() {
    let asked = vec![ask(0), ask(1)];
    let (mut p, first) = recorded(
        asked.clone(),
        vec![page(vec![parent(&asked[0], 17), reply(0)]), fail()],
    );
    p.cfg.review.handle = "private_handle_canary".into();
    p.cfg.token = "private_dispatch_token_canary".into();
    p.data.me.as_mut().unwrap().team = "private_workspace_name_canary".into();
    p.data.me.as_mut().unwrap().user = "private_user_name_canary".into();
    p.data.me.as_mut().unwrap().url = "https://private-workspace.example/".into();
    p.data
        .users
        .insert(BOB.into(), "private_display_name_canary".into());
    let (action, task) = take_task(p.read());
    identity(&action, "T000ACME1", "workspace", "workspace");
    assert!(
        first.requests().is_empty(),
        "no target is observed before its task runs"
    );
    p.data.me.as_mut().unwrap().team_id = "T000NEXT1".into();
    p.cfg.token = "private_reconfigured_token_canary".into();
    for a in &mut p.data.asked {
        a.channel = SVC.into();
        a.ts = format!("{}.000000", NOW + 10);
    }
    let held = p.data.asked.clone();
    let second = Recorded::new(vec![]);
    p.via = crate::Via(Some(second.clone()));
    let fx = run(&mut p, task);
    report(&fx, Outcome::Partial);
    target(&fx, 0, &asked[0], Outcome::Succeeded, None, false, Some(2));
    target(
        &fx,
        1,
        &asked[1],
        Outcome::Failed,
        Some(Reason::Unspecified),
        false,
        None,
    );
    for (a, _) in observations(&fx) {
        if [ATTEMPTED, PARENTS, ERRORS].contains(&a.kind.as_str()) {
            identity(a, "T000ACME1", "workspace", "workspace");
        } else {
            assert_eq!(a.subjects[0].key.instance, "T000ACME1");
            assert!(a.subjects[0].key.id.starts_with(&format!("{TEAM}/")));
        }
    }
    assert_eq!(
        requests(&first),
        asked
            .iter()
            .map(|a| (a.channel.clone(), a.ts.clone()))
            .collect::<Vec<_>>()
    );
    assert!(second.requests().is_empty());
    assert_eq!(
        p.data.asked, held,
        "the captured result does not update a different cache target"
    );
    let safe = format!("{action:?} {:?}", observations(&fx));
    for private in [
        "private_",
        "private.example",
        "private-workspace.example",
        BOB,
        "T000NEXT1",
        SVC,
    ] {
        assert!(!safe.contains(private), "must not retain {private}");
    }
}
