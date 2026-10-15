//! Recorded transport coverage of the real read/task/result pipeline.
//! Generic For-you tasks are deliberately dropped, never polled.

use std::collections::BTreeSet;
use std::sync::Arc;

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome, SubjectRole};
use radar_core::plugin::{Effect, Task};
use serde_json::{Value, json};

use crate::SlackPlugin;
use crate::client::{ChannelInfo, Message, Recorded, Reply};
use crate::demo::{ALICE, BOB, NOW, SVC, TEAM, msg, sample};
use crate::model::ChannelData;

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

fn recorded(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let rec = Recorded::new(replies);
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    p.cfg.channels.truncate(1);
    p.data.channels.clear();
    p.data.channels.insert(
        TEAM.into(),
        ChannelData {
            info: Some(ChannelInfo {
                id: TEAM.into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    p.data.asked.clear();
    (p, rec)
}

/// Taking only the requested task keeps unrelated For-you fixtures unnecessary.
fn take_task(fx: Vec<Effect>, kind: &str) -> (ActionDescriptor, Task) {
    let mut tasks = fx.into_iter().filter_map(|effect| match effect {
        Effect::SpawnAction { action, task } if action.kind.as_str() == kind => {
            Some((action, task))
        }
        _ => None,
    });
    let (action, task) = tasks.next().expect("one semantic task");
    assert_eq!(action.kind.as_str(), kind);
    assert!(
        tasks.next().is_none(),
        "one channel or one batched threads task"
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

fn read_channel(p: &mut SlackPlugin) -> Vec<Effect> {
    let (_, task) = take_task(p.read(), "slack.sync.channel");
    run(p, task)
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

fn observations(fx: &[Effect]) -> Vec<(&ActionDescriptor, Outcome)> {
    fx.iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((action, *outcome)),
            _ => None,
        })
        .collect()
}

fn count(fx: &[Effect], kind: &str, n: u32, outcome: Outcome, truncated: bool) {
    let matching: Vec<_> = observations(fx)
        .into_iter()
        .filter(|(action, _)| action.kind.as_str() == kind)
        .collect();
    assert_eq!(matching.len(), 1, "{kind}: {fx:?}");
    let (action, got) = matching[0];
    assert_eq!(
        (action.metadata.count, got, action.metadata.truncated),
        (Some(n), outcome, truncated),
        "{kind}"
    );
}

fn absent(fx: &[Effect], kind: &str) {
    assert!(
        !observations(fx)
            .iter()
            .any(|(action, _)| action.kind.as_str() == kind),
        "{kind}: {fx:?}"
    );
}

fn failed(fx: &[Effect]) {
    report(fx, Outcome::Failed);
    count(fx, "slack.sync.errors", 1, Outcome::Failed, false);
    assert_eq!(
        observations(fx).len(),
        1,
        "an error returns no partial counts"
    );
    assert!(
        !fx.iter()
            .any(|effect| matches!(effect, Effect::SpawnAction { .. }))
    );
    assert!(!format!("{:?}", observations(fx)).contains("private_error_canary"));
}

fn identity(action: &ActionDescriptor, team: &str, channel: &str) {
    assert_eq!(action.subjects.len(), 1);
    let subject = &action.subjects[0];
    assert_eq!(subject.key.provider, "slack");
    assert_eq!(subject.key.instance, team);
    assert_eq!(subject.key.kind, "channel");
    assert_eq!(subject.key.id, channel);
    assert_eq!(subject.role, SubjectRole::Primary);
    assert!(subject.url.is_none());
}

fn parent(i: usize) -> Value {
    json!({
        "ts": format!("{}.000000", NOW - 1000 + i as i64),
        "user": BOB,
        "text": "private_parent_canary",
        "reply_count": 1,
        "latest_reply": format!("{}.000000", NOW - 500 + i as i64),
    })
}

fn reply(i: usize) -> Value {
    json!({
        "ts": format!("{}.000000", NOW - 500 + i as i64),
        "user": BOB,
        "text": "private_reply_canary",
    })
}

#[test]
fn empty_history_is_success_but_info_and_history_errors_do_not_invent_empty_lists() {
    let (mut p, rec) = recorded(vec![
        ok(json!({ "ok": true, "channel": { "id": TEAM, "name": "private_channel_canary" } })),
        page(vec![]),
    ]);
    p.data.channels.get_mut(TEAM).unwrap().info = None;
    let fx = read_channel(&mut p);
    report(&fx, Outcome::Succeeded);
    count(
        &fx,
        "slack.sync.channel.messages",
        0,
        Outcome::Succeeded,
        false,
    );
    assert_eq!(observations(&fx).len(), 1);
    assert!(p.data.channels[TEAM].messages.is_empty());
    assert_eq!(
        rec.requests().iter().map(|r| r.method).collect::<Vec<_>>(),
        ["conversations.info", "conversations.history"]
    );

    for info_error in [true, false] {
        let (mut p, rec) = recorded(vec![fail()]);
        let held = msg("1791000000.000000", BOB, "retained_cached_message");
        let channel = p.data.channels.get_mut(TEAM).unwrap();
        channel.messages.push(held.clone());
        if info_error {
            channel.info = None;
        }
        let fx = read_channel(&mut p);
        failed(&fx);
        assert_eq!(p.data.channels[TEAM].messages, [held]);
        assert_eq!(rec.requests().len(), 1);
        assert_eq!(
            rec.requests()[0].method,
            if info_error {
                "conversations.info"
            } else {
                "conversations.history"
            }
        );
    }
}

#[test]
fn filtering_join_leave_rows_preserves_the_observed_cutoff() {
    for overflow in [false, true] {
        let mut rows: Vec<_> = (0..299).map(|i| json!({
            "ts": format!("{}.000000", NOW - 1000 + i),
            "subtype": (["channel_join", "channel_leave", "group_join", "group_leave"][i as usize % 4]),
            "user": "UJOINONLY",
        })).collect();
        rows.push(json!({ "ts": "1791270999.000000", "user": BOB, "text": "kept" }));
        if overflow {
            rows.push(json!({ "ts": "1791270999.000001", "text": "discarded overflow" }));
        }
        let (mut p, rec) = recorded(vec![page(rows)]);
        let fx = read_channel(&mut p);
        let outcome = if overflow {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        report(&fx, outcome);
        count(&fx, "slack.sync.channel.messages", 1, outcome, overflow);
        if overflow {
            count(&fx, "slack.sync.lists.truncated", 1, Outcome::Partial, true);
        } else {
            absent(&fx, "slack.sync.lists.truncated");
        }
        assert_eq!(p.data.channels[TEAM].messages.len(), 1);
        assert_eq!(p.data.channels[TEAM].messages[0].text, "kept");
        assert_eq!(
            rec.requests().len(),
            1,
            "filtered-only authors are not looked up"
        );
    }
}

#[test]
fn twenty_versus_twenty_one_stale_threads_share_one_task_and_exclude_parents() {
    for n in [20, 21] {
        let mut replies = vec![page((0..n).map(parent).collect())];
        replies.extend(
            (0..n)
                .rev()
                .take(20)
                .map(|i| page(vec![parent(i), reply(i)])),
        );
        let (mut p, rec) = recorded(replies);
        let channel_fx = read_channel(&mut p);
        report(&channel_fx, Outcome::Succeeded);
        count(
            &channel_fx,
            "slack.sync.channel.messages",
            n as u32,
            Outcome::Succeeded,
            false,
        );
        let (_, task) = take_task(channel_fx, "slack.sync.threads");
        let fx = run(&mut p, task);
        report(
            &fx,
            if n == 20 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
        );
        count(
            &fx,
            "slack.sync.threads.returned",
            20,
            Outcome::Succeeded,
            false,
        );
        count(
            &fx,
            "slack.sync.threads.replies",
            20,
            Outcome::Succeeded,
            false,
        );
        if n == 21 {
            count(
                &fx,
                "slack.sync.threads.deferred",
                1,
                Outcome::Blocked,
                false,
            );
        } else {
            absent(&fx, "slack.sync.threads.deferred");
        }
        assert_eq!(p.data.channels[TEAM].replies.len(), 20);
        assert!(
            p.data.channels[TEAM]
                .replies
                .iter()
                .all(|(ts, replies)| replies.len() == 1 && replies[0].ts != *ts)
        );
        let requests = rec.requests();
        assert_eq!(requests.len(), 21);
        let selected: Vec<_> = requests
            .iter()
            .skip(1)
            .map(|r| {
                assert_eq!(r.method, "conversations.replies");
                r.params
                    .iter()
                    .find(|(key, _)| key == "ts")
                    .unwrap()
                    .1
                    .clone()
            })
            .collect();
        let expected: Vec<_> = (0..n)
            .rev()
            .take(20)
            .map(|i| parent(i)["ts"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            selected, expected,
            "newest stale threads are selected first"
        );
    }
}

#[test]
fn later_thread_error_discards_earlier_results_and_preserves_cached_replies() {
    let (mut p, rec) = recorded(vec![
        page(vec![parent(0), parent(1)]),
        page(vec![parent(1), reply(1)]),
        fail(),
    ]);
    let key = parent(1)["ts"].as_str().unwrap().to_owned();
    let old = msg("1791000000.000000", BOB, "old_cached_reply");
    p.data
        .channels
        .get_mut(TEAM)
        .unwrap()
        .replies
        .insert(key.clone(), vec![old.clone()]);
    let channel_fx = read_channel(&mut p);
    let (_, task) = take_task(channel_fx, "slack.sync.threads");
    let fx = run(&mut p, task);
    failed(&fx);
    assert_eq!(p.data.channels[TEAM].replies.len(), 1);
    assert_eq!(p.data.channels[TEAM].replies[&key], [old]);
    assert_eq!(
        rec.requests().iter().map(|r| r.method).collect::<Vec<_>>(),
        [
            "conversations.history",
            "conversations.replies",
            "conversations.replies"
        ]
    );
}

#[test]
fn forty_versus_forty_one_unknown_names_are_bounded_in_both_task_types() {
    for in_threads in [false, true] {
        for n in [40, 41] {
            let ids: Vec<_> = (0..n).map(|i| format!("UUNKNOWN{i:02}")).collect();
            let mentions = ids
                .iter()
                .map(|id| format!("<@{id}>"))
                .collect::<Vec<_>>()
                .join(" ");
            let mut named = reply(0);
            named["text"] = Value::String(mentions);
            let mut replies = if in_threads {
                vec![page(vec![parent(0)]), page(vec![parent(0), named])]
            } else {
                vec![page(vec![named])]
            };
            replies.extend(ids.iter().take(40).map(|id| ok(json!({
                "ok": true, "user": { "id": id, "profile": { "display_name": "private_name_canary" } }
            }))));
            let (mut p, rec) = recorded(replies);
            let known_before = p.data.users.len();
            let mut fx = read_channel(&mut p);
            if in_threads {
                report(&fx, Outcome::Succeeded);
                absent(&fx, "slack.sync.names.attempted");
                let (_, task) = take_task(fx, "slack.sync.threads");
                fx = run(&mut p, task);
            }
            report(
                &fx,
                if n == 40 {
                    Outcome::Succeeded
                } else {
                    Outcome::Partial
                },
            );
            count(
                &fx,
                "slack.sync.names.attempted",
                40,
                Outcome::Succeeded,
                false,
            );
            count(
                &fx,
                "slack.sync.names.returned",
                40,
                Outcome::Succeeded,
                false,
            );
            absent(&fx, "slack.sync.names.errors");
            if n == 41 {
                count(&fx, "slack.sync.names.deferred", 1, Outcome::Blocked, false);
            } else {
                absent(&fx, "slack.sync.names.deferred");
            }
            let kind = if in_threads {
                "slack.sync.threads.replies"
            } else {
                "slack.sync.channel.messages"
            };
            count(&fx, kind, 1, Outcome::Succeeded, false);
            assert_eq!(p.data.users.len(), known_before + 40);
            assert_eq!(
                rec.requests()
                    .iter()
                    .filter(|r| r.method == "users.info")
                    .count(),
                40
            );
            let safe = format!("{:?}", observations(&fx));
            assert!(!safe.contains("UUNKNOWN") && !safe.contains("private_name_canary"));
        }
    }
}

#[test]
fn name_error_counts_attempts_and_deferrals_without_degrading_a_complete_message_list() {
    let (mut p, rec) = recorded(vec![
        page(vec![
            json!({ "ts": "1791270000.000000", "user": "UUNKNOWN00", "reply_users": ["UUNKNOWN01", "UUNKNOWN02", "WUNKNOWN03", ALICE, "BIGNORE", "SIGNORE"], "text": "<@UUNKNOWN00> <@UUNKNOWN01>" }),
            json!({ "ts": "1791270001.000000", "user": ALICE, "text": "already known" }),
            json!({ "ts": "1791270002.000000", "user": "BNOTUSER", "text": "non-user author" }),
        ]),
        ok(json!({ "ok": true, "user": { "id": "UUNKNOWN00", "name": "private_name_canary" } })),
        fail(),
    ]);
    let fx = read_channel(&mut p);
    report(&fx, Outcome::Partial);
    count(
        &fx,
        "slack.sync.channel.messages",
        3,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.names.attempted",
        2,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.names.returned",
        1,
        Outcome::Succeeded,
        false,
    );
    count(&fx, "slack.sync.names.errors", 1, Outcome::Failed, false);
    count(&fx, "slack.sync.names.deferred", 2, Outcome::Blocked, false);
    assert_eq!(p.data.channels[TEAM].messages.len(), 3);
    assert!(
        p.sync.errors.is_empty(),
        "name enrichment failure is not a history failure"
    );
    let requests = rec.requests();
    let looked_up: Vec<_> = requests
        .iter()
        .filter(|r| r.method == "users.info")
        .map(|r| {
            r.params
                .iter()
                .find(|(key, _)| key == "user")
                .unwrap()
                .1
                .as_str()
        })
        .collect();
    assert_eq!(looked_up, ["UUNKNOWN00", "UUNKNOWN01"]);
    assert!(p.data.users.contains_key("UUNKNOWN00"));
    assert!(!p.data.users.contains_key("UUNKNOWN01"));
}

#[test]
fn dispatch_identity_survives_changes_and_dependent_threads_capture_their_own_dispatch_team() {
    let (mut p, first) = recorded(vec![page(vec![parent(0)])]);
    let (channel_action, channel_task) = take_task(p.read(), "slack.sync.channel");
    identity(&channel_action, "T000ACME1", TEAM);
    p.cfg.channels[0].id = SVC.into();
    p.cfg.channels[0].alias = "private_alias_canary".into();
    p.cfg.token = "private_token_canary".into();
    p.data.me.as_mut().unwrap().team_id = "T000NEXT1".into();
    let second = Recorded::new(vec![page(vec![parent(0), reply(0)])]);
    p.via = crate::Via(Some(second.clone()));
    let channel_fx = run(&mut p, channel_task);
    for (action, _) in observations(&channel_fx) {
        identity(action, "T000ACME1", TEAM);
    }
    let (threads_action, threads_task) = take_task(channel_fx, "slack.sync.threads");
    identity(&threads_action, "T000NEXT1", TEAM);
    p.data.me.as_mut().unwrap().team_id = "T000LAST1".into();
    let fx = run(&mut p, threads_task);
    for (action, _) in observations(&fx) {
        identity(action, "T000NEXT1", TEAM);
    }
    assert_eq!(first.requests()[0].method, "conversations.history");
    assert_eq!(second.requests()[0].method, "conversations.replies");
    for request in first.requests().into_iter().chain(second.requests()) {
        assert_eq!(
            request
                .params
                .iter()
                .find(|(key, _)| key == "channel")
                .unwrap()
                .1,
            TEAM
        );
    }
    let safe = format!(
        "{channel_action:?} {threads_action:?} {:?}",
        observations(&fx)
    );
    for private in [
        "private_alias_canary",
        "private_token_canary",
        "private_parent_canary",
        "private_reply_canary",
        BOB,
    ] {
        assert!(!safe.contains(private), "must not retain {private}");
    }
}

/// The keys of a cache object, less the points a read is kept from (when a
/// channel was read, its whole window, For you, the groups): those are
/// meant to be saved, evidence never.
fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| !["read_at", "swept_at", "for_you_at", "for_you_swept_at", "groups_at"].contains(&k.as_str()))
        .cloned()
        .collect()
}

#[test]
fn transient_channel_and_thread_evidence_never_changes_the_cache_shape() {
    let (mut p, _) = recorded(vec![
        ok(json!({ "ok": true, "messages": [parent(0)], "has_more": true })),
        ok(
            json!({ "ok": true, "messages": [parent(0), reply(0)], "response_metadata": { "next_cursor": null } }),
        ),
    ]);
    p.data
        .channels
        .get_mut(TEAM)
        .unwrap()
        .messages
        .push(serde_json::from_value::<Message>(parent(0)).unwrap());
    let before = p.snapshot().unwrap();
    let channel_fx = read_channel(&mut p);
    report(&channel_fx, Outcome::Partial);
    count(
        &channel_fx,
        "slack.sync.pages.incomplete",
        1,
        Outcome::Partial,
        false,
    );
    let (_, task) = take_task(channel_fx, "slack.sync.threads");
    let fx = run(&mut p, task);
    report(&fx, Outcome::Partial);
    count(
        &fx,
        "slack.sync.pages.incomplete",
        1,
        Outcome::Partial,
        false,
    );
    let after = p.snapshot().unwrap();
    assert_eq!(keys(&before), keys(&after));
    assert_eq!(
        keys(super::tests::account_data(&before)),
        keys(super::tests::account_data(&after))
    );
    assert_eq!(
        keys(&super::tests::account_data(&before)["channels"][TEAM]),
        keys(&super::tests::account_data(&after)["channels"][TEAM])
    );
    assert_eq!(
        keys(&super::tests::account_data(&before)["channels"][TEAM]["messages"][0]),
        keys(&super::tests::account_data(&after)["channels"][TEAM]["messages"][0])
    );
    let serialized = after.to_string();
    for transient in [
        "evidence",
        "names_evidence",
        "incomplete_pages",
        "cutoffs",
        "deferred",
        "attempted",
        "truncated",
        "action",
        "merged",
        "channel_reads",
    ] {
        assert!(
            !serialized.contains(&format!("\"{transient}\"")),
            "cache contains {transient}"
        );
    }
    let mut restored = SlackPlugin::new();
    restored.restore(after.clone());
    assert!(!restored.authenticated());
    assert_eq!(restored.data, crate::model::Data::default());
    assert_eq!(restored.snapshot(), Some(after));
}

#[test]
fn incomplete_successful_collection_is_partial_without_invented_messages_or_a_cutoff() {
    let (mut p, rec) = recorded(vec![ok(json!({
        "ok": true, "response_metadata": { "next_cursor": 7 }
    }))]);
    let fx = read_channel(&mut p);
    report(&fx, Outcome::Partial);
    count(
        &fx,
        "slack.sync.channel.messages",
        0,
        Outcome::Partial,
        false,
    );
    count(
        &fx,
        "slack.sync.pages.incomplete",
        1,
        Outcome::Partial,
        false,
    );
    absent(&fx, "slack.sync.lists.truncated");
    absent(&fx, "slack.sync.errors");
    assert!(p.data.channels[TEAM].messages.is_empty());
    assert_eq!(rec.requests().len(), 1);
}

#[test]
fn bounded_reply_lists_count_returned_replies_after_removing_the_parent() {
    for overflow in [false, true] {
        let mut rows = vec![parent(0)];
        rows.extend((0..199).map(reply));
        if overflow {
            rows.push(reply(199));
        }
        let (mut p, rec) = recorded(vec![page(vec![parent(0)]), page(rows)]);
        let channel_fx = read_channel(&mut p);
        let (_, task) = take_task(channel_fx, "slack.sync.threads");
        let fx = run(&mut p, task);
        let outcome = if overflow {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        report(&fx, outcome);
        count(&fx, "slack.sync.threads.returned", 1, outcome, false);
        count(&fx, "slack.sync.threads.replies", 199, outcome, overflow);
        if overflow {
            count(&fx, "slack.sync.lists.truncated", 1, Outcome::Partial, true);
        } else {
            absent(&fx, "slack.sync.lists.truncated");
        }
        let key = parent(0)["ts"].as_str().unwrap().to_owned();
        let held = &p.data.channels[TEAM].replies[&key];
        assert_eq!(held.len(), 199);
        assert!(held.iter().all(|message| message.ts != key));
        assert_eq!(rec.requests().len(), 2);
    }
}
