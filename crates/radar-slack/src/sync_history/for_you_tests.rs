//! Recorded For-you task/result coverage. Only this task is polled; no provider
//! calls, watched-channel tasks, or followed-review tasks run in these tests.

use std::collections::BTreeSet;
use std::sync::Arc;

use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome, SubjectRole};
use radar_core::plugin::{Effect, Task};
use serde_json::{Value, json};

use crate::SlackPlugin;
use crate::client::{Found, Recorded, Reply, Request, UserGroup};
use crate::demo::{ALICE, BOB, GROUP, NOW, TEAM, msg, sample};
use crate::read::{FOR_YOU_DAYS, GROUPS_EVERY, GROUPS_RETRY};

const ACTION: &str = "slack.sync.for_you";
const MESSAGE: &str = "slack.sync.for_you.message.observed";
const WORKSPACE: &str = "T000ACME1";

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn page(matches: Vec<Value>) -> anyhow::Result<Reply> {
    ok(json!({
        "ok": true,
        "messages": { "paging": { "pages": 1, "total": matches.len() }, "matches": matches }
    }))
}

fn fail() -> anyhow::Result<Reply> {
    ok(json!({ "ok": false, "error": "private_error_canary" }))
}

fn group(id: &str, member: bool) -> UserGroup {
    UserGroup {
        id: id.into(),
        handle: "private_group_handle_canary".into(),
        name: "private_group_name_canary".into(),
        users: vec![if member { ALICE } else { BOB }.into()],
    }
}

fn row(i: usize) -> Value {
    json!({
        "ts": format!("{}.000000", NOW - 1000 + i as i64),
        "user": BOB,
        "text": format!("private_message_canary_{i}"),
        "channel": { "id": TEAM, "name": "private_channel_canary" },
        "permalink": "https://private.example/message_canary"
    })
}

fn expected(i: usize) -> Found {
    Found {
        channel_id: TEAM.into(),
        channel_name: "private_channel_canary".into(),
        permalink: "https://private.example/message_canary".into(),
        message: msg(
            &format!("{}.000000", NOW - 1000 + i as i64),
            BOB,
            &format!("private_message_canary_{i}"),
        ),
        ..Default::default()
    }
}

fn recorded(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let rec = Recorded::new(replies);
    let mut p = sample();
    p.via = crate::Via(Some(rec.clone()));
    p.cfg.channels.clear();
    p.data.channels.clear();
    p.data.asked.clear();
    // A fresh nonempty cache with no memberships avoids an unrelated group read.
    p.data.groups = vec![group("SCACHEDOTHER", false)];
    (p, rec)
}

fn take_task(fx: Vec<Effect>) -> (ActionDescriptor, Task) {
    let mut selected = None;
    for effect in fx {
        match effect {
            Effect::SpawnAction { action, task } if action.kind.as_str() == ACTION => {
                assert!(selected.is_none(), "one workspace task for every query");
                selected = Some((action, task));
            }
            Effect::Spawn(_) | Effect::SpawnAction { .. } => {
                panic!("the fixture must not schedule unrelated reads")
            }
            Effect::Observe { .. } | Effect::Report(_) => {
                panic!("returned evidence must not be fabricated at dispatch")
            }
            _ => {}
        }
    }
    selected.expect("one semantic For-you task")
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

fn report(fx: &[Effect], expected: Outcome) {
    let reports: Vec<_> = fx
        .iter()
        .filter_map(|effect| match effect {
            Effect::Report(outcome) => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(reports, [expected]);
    assert!(
        fx.iter()
            .any(|effect| matches!(effect, Effect::PersistCache))
    );
    assert!(
        !fx.iter()
            .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
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
        .filter(|(action, _)| action.kind.as_str() == kind)
        .collect()
}

fn count(fx: &[Effect], kind: &str, n: u32, outcome: Outcome, truncated: bool) {
    let matching = matching(fx, kind);
    assert_eq!(matching.len(), 1, "{kind}: {fx:?}");
    let (action, got) = matching[0];
    assert_eq!(
        (action.metadata.count, got, action.metadata.truncated),
        (Some(n), outcome, truncated),
        "{kind}"
    );
}

fn absent(fx: &[Effect], kind: &str) {
    assert!(matching(fx, kind).is_empty(), "{kind}: {fx:?}");
}

fn search(query: &str, page: usize) -> Request {
    Request {
        method: "search.messages",
        params: vec![
            ("query".into(), query.into()),
            ("count".into(), "40".into()),
            ("sort".into(), "timestamp".into()),
            ("sort_dir".into(), "desc".into()),
            ("page".into(), page.to_string()),
        ],
        write: false,
        body: None,
    }
}

fn queries(now: i64, groups: &[&str]) -> Vec<String> {
    let after = crate::view::date(now - FOR_YOU_DAYS as i64 * 86_400);
    let mut out = vec![
        format!("<@{ALICE}> after:{after}"),
        format!("to:me after:{after}"),
    ];
    out.extend(
        groups
            .iter()
            .map(|id| format!("<!subteam^{id}> after:{after}")),
    );
    out
}

fn group_request() -> Request {
    Request {
        method: "usergroups.list",
        params: vec![("include_users".into(), "true".into())],
        write: false,
        body: None,
    }
}

fn name_request(id: &str) -> Request {
    Request {
        method: "users.info",
        params: vec![("user".into(), id.into())],
        write: false,
        body: None,
    }
}

fn workspace(action: &ActionDescriptor, team: &str) {
    assert_eq!(action.subjects.len(), 1);
    let subject = &action.subjects[0];
    assert_eq!(subject.key.provider, "slack");
    assert_eq!(subject.key.instance, team);
    assert_eq!(subject.key.kind, "workspace");
    assert_eq!(subject.key.id, "workspace");
    assert_eq!(subject.role, SubjectRole::Primary);
    assert!(subject.url.is_none());
}

#[test]
fn cached_memberships_keep_query_order_duplicates_and_exact_requests() {
    let (mut p, rec) = recorded((0..5).map(|i| page(vec![row(i)])).collect());
    p.data.groups = vec![
        group("SGROUPONE", true),
        group("SNOTMINE", false),
        group("SGROUPONE", true),
        group("SGROUPTWO", true),
    ];
    let groups = p.data.groups.clone();
    let (action, task) = take_task(p.read());
    workspace(&action, WORKSPACE);
    assert!(
        rec.requests().is_empty(),
        "dispatch never runs provider I/O"
    );
    assert_eq!(p.sync.groups_at, NOW);
    let fx = run(&mut p, task);
    report(&fx, Outcome::Succeeded);
    assert_eq!(p.data.groups, groups);
    assert_eq!(
        p.data.for_you,
        (0..5).rev().map(expected).collect::<Vec<_>>()
    );
    assert_eq!(
        rec.requests(),
        queries(NOW, &["SGROUPONE", "SGROUPONE", "SGROUPTWO"])
            .iter()
            .map(|q| search(q, 1))
            .collect::<Vec<_>>()
    );
    assert_eq!(matching(&fx, MESSAGE).len(), 5);
    for suffix in ["planned", "attempted", "returned"] {
        count(
            &fx,
            &format!("slack.sync.for_you.queries.{suffix}"),
            5,
            Outcome::Succeeded,
            false,
        );
    }
    count(
        &fx,
        "slack.sync.for_you.groups.cached",
        3,
        Outcome::Succeeded,
        false,
    );
    absent(&fx, "slack.sync.for_you.groups.attempted");
    absent(&fx, "slack.sync.for_you.queries.deferred");
    let group_searches = matching(&fx, "slack.sync.for_you.search.group");
    assert_eq!(
        group_searches.len(),
        3,
        "duplicate memberships are still separately queried"
    );
    assert!(
        group_searches
            .iter()
            .all(|(a, o)| a.metadata.count == Some(1) && *o == Outcome::Succeeded)
    );
}

#[test]
fn duplicates_within_and_across_queries_keep_first_match_and_channel_identity() {
    let first = row(1);
    let mut dm = row(1);
    dm["channel"]["id"] = "D000DIRECT1".into();
    dm["channel"]["is_im"] = true.into();
    let mut duplicate = row(1);
    duplicate["text"] = "later_duplicate_must_not_replace_first".into();
    let (mut p, rec) = recorded(vec![
        page(vec![first.clone(), first, dm, row(2)]),
        page(vec![row(2), row(3), duplicate]),
    ]);
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    let mut expected_dm = expected(1);
    expected_dm.channel_id = "D000DIRECT1".into();
    expected_dm.is_im = true;
    assert_eq!(
        p.data.for_you,
        [expected(3), expected(2), expected(1), expected_dm]
    );
    assert_eq!(matching(&fx, MESSAGE).len(), 4);
    count(
        &fx,
        "slack.sync.for_you.messages.returned",
        7,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.duplicates",
        3,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.unique",
        4,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.retained",
        4,
        Outcome::Succeeded,
        false,
    );
    absent(&fx, "slack.sync.for_you.messages.omitted");
    assert_eq!(rec.requests().len(), 2);
}

#[test]
fn exactly_eighty_is_complete_but_eighty_one_keeps_only_newest_eighty() {
    for n in [80, 81] {
        let (mut p, rec) = recorded(vec![
            page((0..40).map(row).collect()),
            page((40..80).map(row).collect()),
            page((80..n).map(row).collect()),
        ]);
        p.data.groups = vec![group(GROUP, true)];
        let fx = read(&mut p);
        report(
            &fx,
            if n == 80 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
        );
        assert_eq!(
            p.data.for_you,
            (n - 80..n).rev().map(expected).collect::<Vec<_>>()
        );
        let messages = matching(&fx, MESSAGE);
        assert_eq!(messages.len(), 80);
        count(
            &fx,
            "slack.sync.for_you.messages.returned",
            n as u32,
            Outcome::Succeeded,
            false,
        );
        count(
            &fx,
            "slack.sync.for_you.messages.unique",
            n as u32,
            Outcome::Succeeded,
            false,
        );
        count(
            &fx,
            "slack.sync.for_you.messages.retained",
            80,
            if n == 80 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
            n == 81,
        );
        if n == 81 {
            count(
                &fx,
                "slack.sync.for_you.messages.omitted",
                1,
                Outcome::Partial,
                true,
            );
        } else {
            absent(&fx, "slack.sync.for_you.messages.omitted");
        }
        absent(&fx, "slack.sync.lists.truncated");
        let keys: BTreeSet<_> = messages
            .iter()
            .map(|(a, _)| a.subjects[0].key.id.clone())
            .collect();
        assert_eq!(keys.len(), 80);
        assert_eq!(rec.requests().len(), 3);
    }
}

#[test]
fn later_search_failure_preserves_feed_but_applies_fresh_groups_and_names() {
    let fresh_groups = vec![group("SFRESH", true)];
    let mut found = row(5);
    found["user"] = "UNEWNAME".into();
    let (mut p, rec) = recorded(vec![
        ok(json!({ "ok": true, "usergroups": fresh_groups })),
        page(vec![found]),
        fail(),
        ok(json!({ "ok": true, "user": { "id": "UNEWNAME", "name": "private_name_canary" } })),
    ]);
    p.sync.groups_at = NOW - GROUPS_EVERY;
    let old_feed = p.data.for_you.clone();
    let fx = read(&mut p);
    report(&fx, Outcome::Failed);
    assert_eq!(
        p.data.for_you, old_feed,
        "a partial feed is never applied after a query error"
    );
    assert_eq!(p.data.groups, fresh_groups);
    assert_eq!(
        p.data.users.get("UNEWNAME").map(String::as_str),
        Some("private_name_canary")
    );
    assert_eq!(p.sync.groups_at, NOW);
    count(
        &fx,
        "slack.sync.for_you.queries.planned",
        3,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.attempted",
        2,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.returned",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.deferred",
        1,
        Outcome::Partial,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.groups.attempted",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.groups.returned",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.groups.selected",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.search.mentions",
        1,
        Outcome::Succeeded,
        false,
    );
    let direct = matching(&fx, "slack.sync.for_you.search.direct");
    assert_eq!(direct.len(), 1);
    assert_eq!(direct[0].1, Outcome::Failed);
    assert_eq!(direct[0].0.metadata.count, None);
    absent(&fx, "slack.sync.for_you.search.group");
    count(
        &fx,
        "slack.sync.names.attempted",
        1,
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
    assert!(p.sync.errors.contains_key(&crate::SyncErrorKey::ForYou));
    let messages = matching(&fx, MESSAGE);
    assert_eq!(
        messages.len(),
        1,
        "earlier returned search results still have evidence"
    );
    assert_eq!(messages[0].1, Outcome::Succeeded);
    assert_eq!(
        messages[0].0.subjects[0].key.id,
        format!("{TEAM}/{}.000000", NOW - 995)
    );
    let q = queries(NOW, &["SFRESH"]);
    assert_eq!(
        rec.requests(),
        [
            group_request(),
            search(&q[0], 1),
            search(&q[1], 1),
            name_request("UNEWNAME")
        ]
    );
}

#[test]
fn first_query_error_stops_searches_and_never_invents_results_or_names() {
    let (mut p, rec) = recorded(vec![fail()]);
    p.data.groups = vec![group(GROUP, true)];
    let before = p.snapshot().unwrap();
    let fx = read(&mut p);
    report(&fx, Outcome::Failed);
    assert_eq!(p.snapshot().unwrap(), before);
    absent(&fx, MESSAGE);
    count(
        &fx,
        "slack.sync.for_you.queries.planned",
        3,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.attempted",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.returned",
        0,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.deferred",
        2,
        Outcome::Partial,
        false,
    );
    for suffix in ["returned", "duplicates", "unique", "retained"] {
        count(
            &fx,
            &format!("slack.sync.for_you.messages.{suffix}"),
            0,
            Outcome::Succeeded,
            false,
        );
    }
    let mentions = matching(&fx, "slack.sync.for_you.search.mentions");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].1, Outcome::Failed);
    assert_eq!(mentions[0].0.metadata.count, None);
    absent(&fx, "slack.sync.for_you.search.direct");
    absent(&fx, "slack.sync.for_you.search.group");
    absent(&fx, "slack.sync.names.attempted");
    assert_eq!(rec.requests(), [search(&queries(NOW, &[GROUP])[0], 1)]);
}

#[test]
fn group_error_uses_cached_memberships_and_retries_after_an_hour_not_a_day() {
    let (mut p, first) = recorded(vec![fail(), page(vec![]), page(vec![]), page(vec![])]);
    p.data.groups = vec![group(GROUP, true)];
    p.sync.groups_at = NOW - GROUPS_EVERY;
    let old_groups = p.data.groups.clone();
    let fx = read(&mut p);
    report(&fx, Outcome::Partial);
    assert_eq!(p.data.groups, old_groups);
    count(
        &fx,
        "slack.sync.for_you.groups.attempted",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.groups.errors",
        1,
        Outcome::Failed,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.groups.fallback",
        1,
        Outcome::Partial,
        false,
    );
    absent(&fx, "slack.sync.for_you.groups.returned");
    let retry_at = NOW - (GROUPS_EVERY - GROUPS_RETRY);
    assert_eq!((p.sync.groups_at, p.data.groups_at), (retry_at, retry_at));
    assert!(p.data.for_you.is_empty());
    let expected_requests = std::iter::once(group_request())
        .chain(queries(NOW, &[GROUP]).iter().map(|q| search(q, 1)))
        .collect::<Vec<_>>();
    assert_eq!(first.requests(), expected_requests);

    p.now = NOW + GROUPS_RETRY - 1;
    let second = Recorded::new(vec![page(vec![]), page(vec![]), page(vec![])]);
    p.via = crate::Via(Some(second.clone()));
    report(&read(&mut p), Outcome::Succeeded);
    assert_eq!(p.sync.groups_at, retry_at);
    assert_eq!(
        second.requests(),
        queries(p.now, &[GROUP])
            .iter()
            .map(|q| search(q, 1))
            .collect::<Vec<_>>()
    );

    p.now = NOW + GROUPS_RETRY;
    let third = Recorded::new(vec![
        ok(json!({ "ok": true, "usergroups": [group("SOTHER", false)] })),
        page(vec![]),
        page(vec![]),
    ]);
    p.via = crate::Via(Some(third.clone()));
    let (_, task) = take_task(p.read());
    assert_eq!(
        p.sync.groups_at, p.now,
        "the existing retry timestamp is marked at dispatch"
    );
    report(&run(&mut p, task), Outcome::Succeeded);
    assert_eq!(third.requests()[0], group_request());
    assert_eq!(third.requests().len(), 3);
}

fn groups_asked(rec: &Recorded) -> usize {
    rec.requests()
        .iter()
        .filter(|r| r.method == "usergroups.list")
        .count()
}

#[test]
fn groups_are_asked_once_a_day_not_every_refresh() {
    // Read three hours ago and kept in the cache (what a restart brings):
    // not asked, and `r` does not force it.
    for full in [false, true] {
        let (mut p, rec) = recorded(vec![page(vec![]), page(vec![])]);
        p.sync.groups_at = 0;
        p.data.groups_at = NOW - 3 * 3600;
        p.sync.full_now = full;
        report(&read(&mut p), Outcome::Succeeded);
        assert_eq!(groups_asked(&rec), 0, "full: {full}");
    }
    // A second short of a day: still not.
    let (mut p, rec) = recorded(vec![page(vec![]), page(vec![])]);
    p.sync.groups_at = NOW - GROUPS_EVERY + 1;
    report(&read(&mut p), Outcome::Succeeded);
    assert_eq!(groups_asked(&rec), 0);
    // A day: asked, and the time kept in the cache.
    let (mut p, rec) = recorded(vec![
        ok(json!({ "ok": true, "usergroups": [group("SDAY", false)] })),
        page(vec![]),
        page(vec![]),
    ]);
    p.sync.groups_at = NOW - GROUPS_EVERY;
    p.data.groups_at = NOW - GROUPS_EVERY;
    report(&read(&mut p), Outcome::Succeeded);
    assert_eq!(groups_asked(&rec), 1);
    assert_eq!(p.data.groups_at, NOW);
    assert_eq!(p.data.groups, vec![group("SDAY", false)]);
}

fn search_of(query: &str, count: &str) -> Request {
    let mut request = search(query, 1);
    request.params[1].1 = count.into();
    request
}

/// What Slack says of a search with `matches` on its first page and `pages` in all.
fn paged(matches: Vec<Value>, pages: usize) -> anyhow::Result<Reply> {
    ok(json!({
        "ok": true,
        "messages": { "paging": { "pages": pages, "total": pages }, "matches": matches }
    }))
}

/// Held from a sweep a minute ago: what the refreshes since ask of Slack is
/// whether anything is newer, not for everything again.
fn swept(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let (mut p, rec) = recorded(replies);
    p.data.for_you = vec![expected(1), expected(0)];
    p.data.for_you_swept_at = NOW - 60;
    (p, rec)
}

#[test]
fn between_sweeps_each_query_asks_for_one_match_and_stops_at_one_already_held() {
    // Slack counts pages by the count asked: three matches are three pages.
    let (mut p, rec) = swept(vec![paged(vec![row(1)], 3), paged(vec![row(0)], 9)]);
    let held = p.data.for_you.clone();
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    assert_eq!(
        rec.requests(),
        queries(NOW, &[])
            .iter()
            .map(|q| search_of(q, "1"))
            .collect::<Vec<_>>()
    );
    assert_eq!(p.data.for_you, held, "nothing newer: what is held stays");
    assert_eq!(p.data.for_you_swept_at, NOW - 60, "a probe is not a sweep");
    count(
        &fx,
        "slack.sync.for_you.queries.returned",
        2,
        Outcome::Succeeded,
        false,
    );
    absent(&fx, "slack.sync.lists.truncated");
}

#[test]
fn a_newer_match_widens_the_ask_and_is_folded_in_ahead_of_what_is_held() {
    let (mut p, rec) = swept(vec![
        paged(vec![row(5)], 2),
        paged(vec![row(5), row(1)], 1),
        paged(vec![row(1)], 1),
    ]);
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    let q = queries(NOW, &[]);
    assert_eq!(
        rec.requests(),
        [
            search_of(&q[0], "1"),
            search_of(&q[0], "20"),
            search_of(&q[1], "1")
        ]
    );
    assert_eq!(
        p.data.for_you,
        [expected(5), expected(1), expected(0)],
        "the new one first, what was held after it"
    );
    count(
        &fx,
        "slack.sync.for_you.messages.returned",
        3,
        Outcome::Succeeded,
        false,
    );
    assert_eq!(p.data.for_you_swept_at, NOW - 60);
}

#[test]
fn what_has_left_the_window_is_let_go_when_a_newer_match_is_folded_in() {
    let (mut p, _) = swept(vec![
        paged(vec![row(5)], 1),
        paged(vec![row(5), row(1)], 1),
        paged(vec![row(1)], 1),
    ]);
    let mut old = expected(0);
    old.message.ts = format!("{}.000000", NOW - (FOR_YOU_DAYS as i64 + 1) * 86_400);
    p.data.for_you = vec![expected(1), expected(0), old];
    read(&mut p);
    assert_eq!(p.data.for_you, [expected(5), expected(1), expected(0)]);
}

#[test]
fn every_fifth_refresh_and_r_ask_for_everything_and_say_when() {
    let refresh = {
        let (p, _) = swept(vec![]);
        p.cfg.refresh_secs as i64
    };
    for (swept_ago, full_now, whole) in [
        (5 * refresh - 1, false, false),
        (5 * refresh, false, true),
        (60, true, true),
    ] {
        let (mut p, rec) = swept(vec![page(vec![row(1)]), page(vec![row(0)]), page(vec![])]);
        p.data.for_you_swept_at = NOW - swept_ago;
        p.sync.full_now = full_now;
        // The probe of a refresh between sweeps finds a match it holds.
        let replies = if whole { vec![] } else { vec![paged(vec![row(1)], 3), paged(vec![row(0)], 3)] };
        if !replies.is_empty() {
            *rec.replies.lock().unwrap() = replies.into();
        }
        read(&mut p);
        let counts: Vec<_> = rec
            .requests()
            .iter()
            .map(|r| r.params[1].1.clone())
            .collect();
        let expect = if whole { "40" } else { "1" };
        assert!(counts.iter().all(|c| c == expect), "{swept_ago} {full_now}: {counts:?}");
        assert_eq!(
            p.data.for_you_swept_at,
            if whole { NOW } else { NOW - swept_ago },
            "{swept_ago} {full_now}"
        );
    }
}

#[test]
fn nothing_held_or_a_new_group_read_asks_for_everything() {
    // Nothing held: nothing to stop at.
    let (mut p, rec) = recorded(vec![page(vec![]), page(vec![])]);
    p.data.for_you.clear();
    p.data.for_you_swept_at = NOW - 60;
    read(&mut p);
    assert!(rec.requests().iter().all(|r| r.params[1].1 == "40"));
    // Memberships may have changed with the groups read: the queries change.
    let (mut p, rec) = swept(vec![
        ok(json!({ "ok": true, "usergroups": [group("SNEW", true)] })),
        page(vec![]),
        page(vec![]),
        page(vec![]),
    ]);
    p.sync.groups_at = NOW - GROUPS_EVERY;
    read(&mut p);
    assert_eq!(groups_asked(&rec), 1);
    assert!(
        rec.requests()
            .iter()
            .filter(|r| r.method == "search.messages")
            .all(|r| r.params[1].1 == "40")
    );
}

#[test]
fn empty_groups_are_retried_without_waiting_an_hour() {
    let (mut p, rec) = recorded(vec![
        ok(json!({ "ok": true, "usergroups": [] })),
        page(vec![]),
        page(vec![]),
        fail(),
        page(vec![]),
        page(vec![]),
    ]);
    p.data.groups.clear();
    report(&read(&mut p), Outcome::Succeeded);
    p.now += 1;
    report(&read(&mut p), Outcome::Partial);
    assert_eq!(p.sync.groups_at, NOW + 1 - (GROUPS_EVERY - GROUPS_RETRY));
    assert_eq!(
        rec.requests()
            .iter()
            .filter(|r| r.method == "usergroups.list")
            .count(),
        2
    );
    assert!(p.data.groups.is_empty());
}

#[test]
fn malformed_groups_keep_legacy_parsing_and_mark_auxiliary_gaps() {
    for body in [
        json!({ "ok": true }),
        json!({ "ok": true, "usergroups": null }),
        json!({ "ok": true, "usergroups": "private_bad_collection_canary" }),
        json!({ "ok": true, "usergroups": [{ "id": "SMALFORMED", "users": null }] }),
        json!({ "ok": true, "usergroups": [{ "id": "SMALFORMED", "users": [ALICE, 7] }] }),
    ] {
        let member = body
            .pointer("/usergroups/0/users/0")
            .and_then(Value::as_str)
            == Some(ALICE);
        let mut replies = vec![ok(body.clone()), page(vec![]), page(vec![])];
        if member {
            replies.push(page(vec![]));
        }
        let (mut p, rec) = recorded(replies);
        p.sync.groups_at = NOW - GROUPS_EVERY;
        let fx = read(&mut p);
        report(&fx, Outcome::Partial);
        let expected_groups = if body["usergroups"].is_array() {
            vec![UserGroup {
                id: "SMALFORMED".into(),
                users: if member { vec![ALICE.into()] } else { vec![] },
                ..Default::default()
            }]
        } else {
            vec![]
        };
        assert_eq!(p.data.groups, expected_groups);
        count(
            &fx,
            "slack.sync.for_you.groups.returned",
            expected_groups.len() as u32,
            Outcome::Partial,
            false,
        );
        count(
            &fx,
            "slack.sync.for_you.groups.selected",
            u32::from(member),
            Outcome::Succeeded,
            false,
        );
        if body["usergroups"].is_array() {
            count(
                &fx,
                "slack.sync.for_you.groups.entries.incomplete",
                1,
                Outcome::Partial,
                false,
            );
            absent(&fx, "slack.sync.for_you.groups.collections.incomplete");
        } else {
            count(
                &fx,
                "slack.sync.for_you.groups.collections.incomplete",
                1,
                Outcome::Partial,
                false,
            );
            absent(&fx, "slack.sync.for_you.groups.entries.incomplete");
        }
        assert_eq!(p.sync.groups_at, NOW);
        assert!(p.data.for_you.is_empty());
        assert_eq!(rec.requests().len(), if member { 4 } else { 3 });
    }
}

#[test]
fn names_have_a_forty_user_bound_without_degrading_message_observations() {
    for n in [40, 41] {
        let ids: Vec<_> = (0..n).map(|i| format!("UUNKNOWN{i:02}")).collect();
        let mut found = row(1);
        found["text"] = ids
            .iter()
            .map(|id| format!("<@{id}>"))
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        let mut replies = vec![page(vec![found]), page(vec![])];
        replies.extend(ids.iter().take(40).map(|id| {
            ok(json!({ "ok": true, "user": { "id": id, "name": "private_name_canary" } }))
        }));
        let (mut p, rec) = recorded(replies);
        let before = p.data.users.len();
        let fx = read(&mut p);
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
        assert_eq!(p.data.users.len(), before + 40);
        assert_eq!(p.data.for_you.len(), 1);
        let messages = matching(&fx, MESSAGE);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1, Outcome::Succeeded);
        assert_eq!(
            rec.requests()
                .iter()
                .filter(|r| r.method == "users.info")
                .cloned()
                .collect::<Vec<_>>(),
            ids.iter()
                .take(40)
                .map(|id| name_request(id))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn name_lookup_stops_at_first_error_and_uses_only_retained_unique_messages() {
    let mut oldest = row(0);
    oldest["user"] = "UDROPPED".into();
    let mut newest = row(80);
    newest["reply_users"] = json!(["UUNKNOWN00", "UUNKNOWN01", "WUNKNOWN02", ALICE, "BIGNORE"]);
    newest["text"] = "<@UUNKNOWN00> <@UUNKNOWN01>".into();
    let mut first_page: Vec<_> = (0..40).map(row).collect();
    first_page[0] = oldest;
    let (mut p, rec) = recorded(vec![
        page(first_page),
        page((40..80).map(row).collect()),
        page(vec![newest]),
        ok(json!({ "ok": true, "user": { "id": "UUNKNOWN00", "name": "private_name_canary" } })),
        fail(),
    ]);
    p.data.groups = vec![group(GROUP, true)];
    let fx = read(&mut p);
    report(&fx, Outcome::Partial);
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
    count(&fx, "slack.sync.names.deferred", 1, Outcome::Blocked, false);
    assert_eq!(p.data.for_you.len(), 80);
    assert!(p.data.users.contains_key("UUNKNOWN00"));
    assert!(!p.data.users.contains_key("UUNKNOWN01"));
    assert!(
        p.sync.errors.is_empty(),
        "name enrichment errors are not feed errors"
    );
    assert_eq!(
        rec.requests()
            .iter()
            .filter(|r| r.method == "users.info")
            .cloned()
            .collect::<Vec<_>>(),
        [name_request("UUNKNOWN00"), name_request("UUNKNOWN01")]
    );
    assert!(
        matching(&fx, MESSAGE)
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Succeeded)
    );
}

#[test]
fn dispatch_workspace_is_captured_even_when_account_and_client_change() {
    let (mut p, rec) = recorded(vec![page(vec![row(1)]), page(vec![])]);
    let (action, task) = take_task(p.read());
    workspace(&action, WORKSPACE);
    p.data.me.as_mut().unwrap().team_id = "T000NEXT1".into();
    p.data.me.as_mut().unwrap().user_id = "UNEWNEXT1".into();
    p.cfg.token = "private_token_canary".into();
    let replacement = Recorded::new(vec![]);
    p.via = crate::Via(Some(replacement.clone()));
    let fx = run(&mut p, task);
    report(&fx, Outcome::Succeeded);
    assert!(replacement.requests().is_empty());
    assert_eq!(
        rec.requests(),
        queries(NOW, &[])
            .iter()
            .map(|q| search(q, 1))
            .collect::<Vec<_>>()
    );
    for (action, _) in observations(&fx) {
        assert!(!action.subjects.is_empty());
        for subject in &action.subjects {
            assert_eq!(subject.key.instance, WORKSPACE);
        }
    }
}

#[test]
fn message_identity_accepts_direct_messages_and_only_explicit_thread_context() {
    let mut threaded = row(1);
    threaded["channel"]["id"] = "D000DIRECT1".into();
    threaded["channel"]["is_im"] = true.into();
    threaded["thread_ts"] = "1790000000.000001".into();
    let mut top_level = row(2);
    top_level["permalink"] = "https://private.example/?thread_ts=1780000000.000001".into();
    top_level["reply_count"] = 3.into();
    let (mut p, _) = recorded(vec![page(vec![threaded, top_level]), page(vec![])]);
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    let messages = matching(&fx, MESSAGE);
    assert_eq!(messages.len(), 2);
    for (action, outcome) in &messages {
        assert_eq!(*outcome, Outcome::Succeeded);
        assert_eq!(action.metadata.count, Some(1));
        assert!(!action.metadata.truncated);
        let result: Vec<_> = action
            .subjects
            .iter()
            .filter(|s| s.role == SubjectRole::Result)
            .collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key.provider, "slack");
        assert_eq!(result[0].key.instance, WORKSPACE);
        assert_eq!(result[0].key.kind, "message");
        assert!(action.subjects.iter().all(|subject| subject.url.is_none()));
    }
    let threaded = messages
        .iter()
        .find(|(a, _)| {
            a.subjects
                .iter()
                .any(|s| s.key.id.starts_with("D000DIRECT1/"))
        })
        .unwrap()
        .0;
    assert_eq!(threaded.subjects.len(), 2);
    let context = threaded
        .subjects
        .iter()
        .find(|s| s.role == SubjectRole::Context)
        .unwrap();
    assert_eq!(context.key.kind, "thread");
    assert_eq!(context.key.id, "D000DIRECT1/1790000000.000001");
    let top_level = messages
        .iter()
        .find(|(a, _)| a.subjects.iter().any(|s| s.key.id.starts_with(TEAM)))
        .unwrap()
        .0;
    assert_eq!(
        top_level.subjects.len(),
        1,
        "a permalink or reply count must not imply thread context"
    );
}

#[test]
fn unsafe_identity_components_are_redacted_without_leaking_payloads() {
    for (_team, channel, ts, thread_ts) in [
        (
            WORKSPACE,
            "private_channel_id_canary",
            "1791000000.000001",
            None,
        ),
        (WORKSPACE, TEAM, "private_timestamp_canary", None),
        (
            WORKSPACE,
            TEAM,
            "1791000000.000001",
            Some("private_thread_canary"),
        ),
    ] {
        let mut found = row(1);
        found["channel"]["id"] = channel.into();
        found["ts"] = ts.into();
        if let Some(thread) = thread_ts {
            found["thread_ts"] = thread.into();
        }
        let (mut p, _) = recorded(vec![page(vec![found]), page(vec![])]);
        let (action, task) = take_task(p.read());
        let fx = run(&mut p, task);
        let messages = matching(&fx, MESSAGE);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1, Outcome::Succeeded);
        assert!(messages[0].0.metadata.redacted);
        assert!(messages[0].0.metadata.omitted_subjects > 0);
        if thread_ts.is_none() {
            assert!(messages[0].0.subjects.is_empty());
        } else {
            assert_eq!(messages[0].0.subjects.len(), 1);
        }
        let safe = format!("{action:?} {:?}", observations(&fx));
        for private in [
            "private_team_canary",
            "private_channel_id_canary",
            "private_timestamp_canary",
            "private_thread_canary",
            "private_message_canary",
            "private_channel_canary",
            "private.example",
            "private_group",
            ALICE,
            BOB,
        ] {
            assert!(!safe.contains(private), "journal evidence leaked {private}");
        }
    }
}

#[test]
fn unsafe_workspace_identity_is_redacted_at_the_evidence_boundary() {
    // An invalid owner cannot dispatch a scoped read. Exercise the pure
    // descriptor boundary independently of authenticated provider requests.
    let action = super::for_you_message("private_team_canary", &expected(1));
    assert!(action.metadata.redacted);
    assert!(action.metadata.omitted_subjects > 0);
    assert!(action.subjects.is_empty());
    let safe = format!("{action:?}");
    for private in [
        "private_team_canary",
        "private_message_canary",
        "private_channel_canary",
        "private.example",
        "private_group",
        ALICE,
        BOB,
    ] {
        assert!(!safe.contains(private), "journal evidence leaked {private}");
    }
}

/// The keys of a cache object, less the points a read is kept from: those
/// are meant to be saved, evidence never.
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
fn transient_evidence_never_changes_serialized_provider_cache_shape() {
    let (mut p, _) = recorded(vec![page(vec![row(1)]), page(vec![])]);
    p.data.for_you = vec![expected(0)];
    let before = p.snapshot().unwrap();
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    let after = p.snapshot().unwrap();
    assert_eq!(keys(&before), keys(&after));
    for path in ["", "/for_you/0", "/for_you/0/message"] {
        assert_eq!(
            keys(super::tests::account_data(&before).pointer(path).unwrap()),
            keys(super::tests::account_data(&after).pointer(path).unwrap()),
            "cache shape at {path}"
        );
    }
    let serialized = after.to_string();
    for transient in [
        "evidence",
        "names_evidence",
        "incomplete_pages",
        "queries",
        "deferred",
        "attempted",
        "truncated",
        "action",
    ] {
        assert!(
            !serialized.contains(&format!("\"{transient}\"")),
            "cache contains {transient}"
        );
    }
    let safe = format!("{:?}", observations(&fx));
    for private in [
        "private_", "https://", ALICE, BOB, "after:", "to:me", "subteam^",
    ] {
        assert!(!safe.contains(private), "evidence leaked {private}");
    }
    for (action, _) in observations(&fx) {
        assert!(action.metadata.url.is_none());
        assert!(action.metadata.command.is_none());
        assert!(action.subjects.iter().all(|s| s.url.is_none()));
    }
    let mut restored = SlackPlugin::new();
    restored.restore(after.clone());
    assert!(!restored.authenticated());
    assert_eq!(restored.data, crate::model::Data::default());
    assert_eq!(restored.snapshot(), Some(after));
}

#[test]
fn search_count_and_five_page_bounds_do_not_issue_extra_requests() {
    for row_bound in [false, true] {
        let mut replies = if row_bound {
            vec![ok(json!({
                "ok": true,
                "messages": { "matches": (0..40).map(row).collect::<Vec<_>>(), "paging": { "pages": 2 } }
            }))]
        } else {
            (0..5)
                .map(|i| {
                    ok(json!({
                        "ok": true,
                        "messages": { "matches": [row(i)], "paging": { "pages": 6 } }
                    }))
                })
                .collect()
        };
        replies.push(page(vec![]));
        let (mut p, rec) = recorded(replies);
        let fx = read(&mut p);
        report(&fx, Outcome::Partial);
        let retained = if row_bound { 40 } else { 5 };
        assert_eq!(
            p.data.for_you,
            (0..retained).rev().map(expected).collect::<Vec<_>>()
        );
        count(
            &fx,
            "slack.sync.for_you.search.mentions",
            retained as u32,
            Outcome::Partial,
            true,
        );
        count(
            &fx,
            "slack.sync.for_you.search.direct",
            0,
            Outcome::Succeeded,
            false,
        );
        count(&fx, "slack.sync.lists.truncated", 1, Outcome::Partial, true);
        absent(&fx, "slack.sync.pages.incomplete");
        absent(&fx, "slack.sync.for_you.messages.omitted");
        let q = queries(NOW, &[]);
        let expected_requests = (1..=if row_bound { 1 } else { 5 })
            .map(|page| search(&q[0], page))
            .chain(std::iter::once(search(&q[1], 1)))
            .collect::<Vec<_>>();
        assert_eq!(rec.requests(), expected_requests);
        assert_eq!(matching(&fx, MESSAGE).len(), retained);
    }
}

#[test]
fn incomplete_search_pages_apply_legacy_empty_results_without_inventing_a_cutoff() {
    for body in [
        json!({ "ok": true }),
        json!({ "ok": true, "messages": { "matches": null } }),
        json!({ "ok": true, "messages": { "matches": [], "paging": { "pages": 2 } } }),
        json!({ "ok": true, "messages": { "matches": [], "paging": { "pages": "private_bad_paging_canary" } } }),
    ] {
        let (mut p, rec) = recorded(vec![ok(body), page(vec![])]);
        assert!(!p.data.for_you.is_empty());
        let fx = read(&mut p);
        report(&fx, Outcome::Partial);
        assert!(
            p.data.for_you.is_empty(),
            "retain the existing successful empty-result cache semantics"
        );
        count(
            &fx,
            "slack.sync.for_you.search.mentions",
            0,
            Outcome::Partial,
            false,
        );
        count(
            &fx,
            "slack.sync.for_you.search.direct",
            0,
            Outcome::Succeeded,
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
        absent(&fx, MESSAGE);
        assert_eq!(
            rec.requests(),
            queries(NOW, &[])
                .iter()
                .map(|q| search(q, 1))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn error_on_later_page_discards_that_querys_unreturned_rows() {
    let mut first_page = row(1);
    first_page["user"] = "UNOTRETURNED".into();
    let (mut p, rec) = recorded(vec![
        ok(
            json!({ "ok": true, "messages": { "matches": [first_page], "paging": { "pages": 2 } } }),
        ),
        fail(),
    ]);
    let before = p.snapshot().unwrap();
    let fx = read(&mut p);
    report(&fx, Outcome::Failed);
    assert_eq!(p.snapshot().unwrap(), before);
    count(
        &fx,
        "slack.sync.for_you.queries.attempted",
        1,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.returned",
        0,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.deferred",
        1,
        Outcome::Partial,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.returned",
        0,
        Outcome::Succeeded,
        false,
    );
    absent(&fx, "slack.sync.names.attempted");
    absent(&fx, MESSAGE);
    let q = queries(NOW, &[]);
    assert_eq!(rec.requests(), [search(&q[0], 1), search(&q[0], 2)]);
    let safe = format!("{:?}", observations(&fx));
    for private in [
        "UNOTRETURNED",
        "private_error_canary",
        "private_message_canary",
    ] {
        assert!(!safe.contains(private));
    }
}

#[test]
fn later_failed_query_still_bounds_retained_result_evidence_to_eighty() {
    let (mut p, rec) = recorded(vec![
        page((0..40).map(row).collect()),
        page((40..80).map(row).collect()),
        page(vec![row(80)]),
        fail(),
    ]);
    p.data.groups = vec![
        group("SFIRST", true),
        group("SSECOND", true),
        group("SDEFERRED", true),
    ];
    let before = p.snapshot().unwrap();
    let fx = read(&mut p);
    report(&fx, Outcome::Failed);
    assert_eq!(p.snapshot().unwrap(), before);
    count(
        &fx,
        "slack.sync.for_you.queries.planned",
        5,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.attempted",
        4,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.returned",
        3,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.queries.deferred",
        1,
        Outcome::Partial,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.unique",
        81,
        Outcome::Succeeded,
        false,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.retained",
        80,
        Outcome::Partial,
        true,
    );
    count(
        &fx,
        "slack.sync.for_you.messages.omitted",
        1,
        Outcome::Partial,
        true,
    );
    let messages = matching(&fx, MESSAGE);
    assert_eq!(messages.len(), 80);
    assert!(
        messages
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Succeeded)
    );
    assert!(!messages.iter().any(|(action, _)| action.subjects[0].key.id == format!("{TEAM}/{}.000000", NOW - 1000)));
    assert_eq!(rec.requests().len(), 4);
}

#[test]
fn successful_profile_calls_are_counted_without_claiming_valid_profile_contents() {
    let mut found = row(1);
    found["user"] = "UNORESULT".into();
    let (mut p, rec) = recorded(vec![
        page(vec![found]),
        page(vec![]),
        ok(json!({ "ok": true })),
    ]);
    let fx = read(&mut p);
    report(&fx, Outcome::Succeeded);
    count(
        &fx,
        "slack.sync.names.attempted",
        1,
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
    absent(&fx, "slack.sync.names.errors");
    assert_eq!(p.data.users.get("UNORESULT").map(String::as_str), Some(""));
    assert_eq!(p.data.for_you.len(), 1);
    assert_eq!(rec.requests().last(), Some(&name_request("UNORESULT")));
    assert!(!format!("{:?}", observations(&fx)).contains("UNORESULT"));
}
