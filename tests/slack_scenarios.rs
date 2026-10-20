//! Renderer-free journeys through the real host, Slack tasks and journal.
//! Provider I/O is scripted; restored cache values are real emitted snapshots.
#![cfg(feature = "slack")]

#[path = "support/slack_checkpoints.rs"]
mod checkpoints;
use checkpoints::*;
#[path = "support/probes.rs"]
mod probes;
use probes::{Probe, require, verify};

use radar_core::host::JournalRecord;
use radar_core::journal::{ActionEvent, Journal, Outcome, Query};
use radar_core::view::Key;
use radar_core::scenario::Scenario;
use radar_core::writes::Mode;
use radar_slack::test_support::{
    AcceptedPost, ExpectedRequest, HostedFixture, Reply, RequestRecord,
};
use serde_json::json;

/// Captured values only: probes cannot dispatch, release replies or mutate a host.
struct PendingReply {
    requests: Vec<RequestRecord>,
    records: Vec<JournalRecord>,
    view: serde_json::Value,
    notice: Option<String>,
    desktop_is_empty: bool,
}

fn pending_reply(s: &Scenario, fixture: &HostedFixture) {
    let checkpoint = PendingReply {
        requests: fixture.ledger(),
        records: s.records().to_vec(),
        view: view(s),
        notice: s.host().message.as_ref().map(ToString::to_string),
        desktop_is_empty: s.desktop_effects().is_empty(),
    };
    verify("reply awaiting provider", &checkpoint, &[
        Probe { name: "exact single held write", check: |c| {
            let writes: Vec<_> = c.requests.iter().filter(|r| r.request.write).collect();
            require(writes.len() == 1 && writes[0].key.as_deref() == Some("post")
                && writes[0].request == post() && !writes[0].completed,
                "expected the exact thread/body request once, still held")
        } },
        Probe { name: "real action is started and nonterminal", check: |c| {
            let id = c.records.iter().find_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == "slack.reply.post" => Some(id),
                _ => None,
            }).ok_or_else(|| "semantic reply action missing".to_owned())?;
            require(c.records.iter().any(|record| matches!(record,
                JournalRecord::Outcome { id: current, outcome: Outcome::Started, .. } if current == id)),
                "reply never started")?;
            require(!c.records.iter().any(|record| matches!(record,
                JournalRecord::Outcome { id: current, outcome, .. } if current == id && outcome.is_terminal())),
                "held response produced a premature terminal outcome")
        } },
        Probe { name: "thread remains visible", check: |c| require(
            c.view.to_string().contains("The Watch needs a gate report"), "original thread disappeared") },
        Probe { name: "no premature success notice", check: |c| require(
            !c.notice.as_deref().unwrap_or("").contains("reply posted"), "success announced before release") },
        Probe { name: "no desktop effects", check: |c| require(c.desktop_is_empty, "desktop operation requested") },
    ]).unwrap_or_else(|error| panic!("{error}"));
}

fn persisted_events(scratch: &Scratch) -> Vec<ActionEvent> {
    Journal::open(&scratch.journal())
        .unwrap()
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap()
}

fn journal(s: &mut Scenario, scratch: &Scratch, expected: Outcome) {
    assert!(
        s.desktop_effects().is_empty(),
        "no desktop I/O was requested"
    );
    s.finish_journal().unwrap();
    let events = persisted_events(scratch);
    struct PersistedReply {
        posts: Vec<ActionEvent>,
        events: Vec<ActionEvent>,
        expected: Outcome,
    }
    let checkpoint = PersistedReply {
        posts: events
            .iter()
            .filter(|e| e.kind.as_str() == "slack.reply.post")
            .cloned()
            .collect(),
        events,
        expected,
    };
    verify(
        "reply persisted",
        &checkpoint,
        &[
            Probe {
                name: "one semantic action",
                check: |c| {
                    let first = c
                        .posts
                        .first()
                        .ok_or_else(|| "semantic reply missing".to_owned())?;
                    require(
                        c.posts.iter().all(|e| e.action_id == first.action_id),
                        "multiple reply actions",
                    )
                },
            },
            Probe {
                name: "request and confirmation retained",
                check: |c| {
                    require(
                        c.posts.iter().any(|e| e.outcome == Outcome::Requested)
                            && c.posts.iter().any(|e| e.outcome == Outcome::Confirmed),
                        "missing request or confirmation",
                    )
                },
            },
            Probe {
                name: "exact terminal outcome once",
                check: |c| {
                    let terminal: Vec<_> = c
                        .posts
                        .iter()
                        .filter(|e| e.outcome.is_terminal())
                        .map(|e| e.outcome)
                        .collect();
                    require(
                        terminal == [c.expected],
                        &format!("expected {:?} once, got {terminal:?}", c.expected),
                    )
                },
            },
            Probe {
                name: "message text excluded",
                check: |c| {
                    require(
                        !serde_json::to_string(&c.events).unwrap().contains(BODY),
                        "journal contains message text",
                    )
                },
            },
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

fn reopen_retained_reply(s: &mut Scenario) {
    key(s, Key::Char('c'));
    key(s, Key::Enter);
    assert!(
        s.host()
            .menu
            .as_ref()
            .expect("retained reply details")
            .menu
            .items
            .iter()
            .any(|row| row.label.contains(BODY))
    );
    key(s, Key::Char('e'));
    assert!(s.host().plugins[0].capturing());
    assert!(
        view(s).to_string().contains(BODY),
        "retained text can be reopened"
    );
}

struct AcceptedPending {
    receipts: Vec<AcceptedPost>,
    requests: Vec<RequestRecord>,
}

fn accepted_pending(fixture: &HostedFixture) -> AcceptedPost {
    let checkpoint = AcceptedPending {
        receipts: fixture.accepted_posts(),
        requests: fixture.ledger(),
    };
    verify(
        "synthetic acceptance before response loss",
        &checkpoint,
        &[
            Probe {
                name: "one exact accepted post",
                check: |c| {
                    require(c.receipts.len() == 1, "expected one observed acceptance")?;
                    require(
                        c.receipts[0].key() == "post"
                            && c.receipts[0].request() == &post()
                            && c.receipts[0].server_ts() == "1791271000.000900",
                        "accepted receipt differs from the post",
                    )
                },
            },
            Probe {
                name: "response remains held",
                check: |c| {
                    require(
                        c.requests
                            .iter()
                            .any(|r| r.key.as_deref() == Some("post") && !r.completed),
                        "acceptance was not observed before response completion",
                    )
                },
            },
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"));
    checkpoint.receipts.into_iter().next().unwrap()
}

struct RestoredBeforeAuth {
    requests: Vec<RequestRecord>,
    view: serde_json::Value,
    status: String,
    notice: String,
    capturing: bool,
    desktop_is_empty: bool,
}

fn restored_before_auth(s: &Scenario, fixture: &HostedFixture) {
    let checkpoint = RestoredBeforeAuth {
        requests: fixture.ledger(),
        view: view(s),
        status: s.host().plugins[0].top_right("feed").unwrap_or_default(),
        notice: s
            .host()
            .message
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        capturing: s.host().plugins[0].capturing(),
        desktop_is_empty: s.desktop_effects().is_empty(),
    };
    verify(
        "restored feed awaiting fresh identity",
        &checkpoint,
        &[
            Probe {
                name: "real cached presentation",
                check: |c| {
                    require(
                        c.status.starts_with("cached")
                            && c.view.to_string().contains("The Watch needs a gate report"),
                        "captured snapshot did not restore the cached feed",
                    )
                },
            },
            Probe {
                name: "cached identity is not authority",
                check: |c| {
                    require(
                        c.requests.len() == 1
                            && c.requests[0].request.method == "auth.test"
                            && !c.requests[0].completed
                            && c.notice.contains("check the Slack connection"),
                        "cached identity bypassed fresh authentication",
                    )
                },
            },
            Probe {
                name: "draft remains session local",
                check: |c| {
                    require(
                        !c.capturing && !c.view.to_string().contains(BODY),
                        "a previous session draft was restored",
                    )
                },
            },
            Probe {
                name: "no desktop effects",
                check: |c| {
                    require(
                        c.desktop_is_empty,
                        "cached checkpoint performed desktop I/O",
                    )
                },
            },
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

struct RestartedThread {
    requests: Vec<RequestRecord>,
    receipts: Vec<AcceptedPost>,
    body: serde_json::Value,
    read_mark: Option<String>,
    accepted_ts: String,
    capturing: bool,
    desktop_is_empty: bool,
}

fn restarted_thread(s: &Scenario, fixture: &HostedFixture, accepted: &AcceptedPost) {
    let body = s.inspect(|host| {
        let screen = host.screen_view_in(0, "feed", 120, 40).unwrap();
        let page = screen.page.expect("fresh Host thread page");
        let pane = page
            .panes
            .iter()
            .find(|p| p.pane == radar_core::view::PaneId::Middle)
            .expect("thread content pane");
        serde_json::to_value(&pane.body).unwrap()
    });
    let cache = CapturedSlackCache::capture(s).value();
    let checkpoint = RestartedThread {
        requests: fixture.ledger(),
        receipts: fixture.accepted_posts(),
        body,
        read_mark: cache["accounts"][0]["data"]["opened"][format!("{CHANNEL}/{ROOT}")]
            .as_str()
            .map(str::to_owned),
        accepted_ts: accepted.server_ts().into(),
        capturing: s.host().plugins[0].capturing(),
        desktop_is_empty: s.desktop_effects().is_empty(),
    };
    verify("accepted post read by a fresh Host", &checkpoint, &[
        Probe { name: "accepted post ingested and shown once", check: |c| require(
            c.requests.iter().filter(|r| r.key.as_deref() == Some("accepted-thread") && r.completed).count() == 1
                && c.body.to_string().matches(BODY).count() == 1
                && c.read_mark.as_deref() == Some(c.accepted_ts.as_str()),
            "provider post was not displayed once and marked read through its accepted timestamp") },
        Probe { name: "no new session resend", check: |c| require(
            c.requests.iter().all(|r| !r.request.write) && c.receipts.is_empty(),
            "fresh session sent or accepted another write") },
        Probe { name: "server text is not a restored draft", check: |c| require(
            !c.capturing, "previous reply composer survived into the fresh Host") },
        Probe { name: "no desktop effects", check: |c| require(
            c.desktop_is_empty, "thread checkpoint performed desktop I/O") },
    ]).unwrap_or_else(|error| panic!("{error}"));
}

#[tokio::test(flavor = "current_thread")]
async fn confirmed_reply_waits_for_provider_then_refreshes_and_persists_success() {
    let scratch = Scratch::new();
    let response = ExpectedRequest::json(
        "post",
        post(),
        json!({"ok":true,"channel":CHANNEL,"ts":"1791271000.000900"}),
    )
    .held();
    let (mut s, fixture) =
        at_open_thread(Mode::Write, vec![response, refreshed_thread()], &scratch).await;
    compose_reply(&mut s, &fixture);
    key(&mut s, Key::Char('y'));
    s.settle_until(BUDGET, |_| writes(&fixture) == 1)
        .await
        .unwrap();
    pending_reply(&s, &fixture);
    fixture.release("post").unwrap();
    s.settle(BUDGET).await.unwrap();
    assert_eq!(writes(&fixture), 1);
    assert!(
        s.host()
            .message
            .as_deref()
            .unwrap()
            .contains("reply posted")
    );
    assert!(!s.host().plugins[0].capturing());
    // The real thread page contains the provider result, not only a success toast.
    assert!(view(&s).to_string().contains(BODY));
    fixture.assert_finished();
    journal(&mut s, &scratch, Outcome::Succeeded);
}

#[tokio::test(flavor = "current_thread")]
async fn read_only_confirmation_refuses_all_writes_and_keeps_the_reply() {
    let scratch = Scratch::new();
    let (mut s, fixture) = at_reply_confirmation(Mode::ReadOnly, vec![], &scratch).await;
    assert!(
        s.host()
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|r| r.label.contains("read only"))
    );
    key(&mut s, Key::Char('y'));
    s.settle(BUDGET).await.unwrap();
    assert_eq!(writes(&fixture), 0);
    assert!(
        s.host()
            .message
            .as_deref()
            .unwrap()
            .contains("reply not sent")
    );
    key(&mut s, Key::Char('c'));
    key(&mut s, Key::Enter);
    assert!(
        s.host()
            .menu
            .as_ref()
            .expect("retained reply details")
            .menu
            .items
            .iter()
            .any(|row| row.label.contains(BODY))
    );
    key(&mut s, Key::Char('e')); // Explicitly reopen the retained text.
    assert!(s.host().plugins[0].capturing());
    assert!(
        view(&s).to_string().contains(BODY),
        "refused reply can be recovered"
    );
    fixture.assert_finished();
    journal(&mut s, &scratch, Outcome::Blocked);
}

#[tokio::test(flavor = "current_thread")]
async fn uncertain_provider_failure_is_not_retried_and_retains_the_draft() {
    let scratch = Scratch::new();
    let response = ExpectedRequest::reply(
        "post",
        post(),
        Reply {
            status: 429,
            retry_after: Some(30),
            ..Default::default()
        },
    )
    .held();
    let (mut s, fixture) = at_reply_confirmation(Mode::Write, vec![response], &scratch).await;
    key(&mut s, Key::Char('y'));
    s.settle_until(BUDGET, |_| writes(&fixture) == 1)
        .await
        .unwrap();
    pending_reply(&s, &fixture);
    fixture.release("post").unwrap();
    s.settle(BUDGET).await.unwrap();
    assert_eq!(
        s.host().message.as_deref(),
        Some(
            "✗ reply outcome uncertain; a post may already exist · c has your text · ctrl-r lists kept replies"
        )
    );
    reopen_retained_reply(&mut s);
    // User recovery and ordinary ticks cannot silently repeat a write.
    s.advance_time(NOW + 1).unwrap();
    s.settle(BUDGET).await.unwrap();
    assert_eq!(writes(&fixture), 1);
    fixture.assert_finished();
    journal(&mut s, &scratch, Outcome::Unknown);
}

#[tokio::test(flavor = "current_thread")]
async fn accepted_post_with_lost_response_stays_unknown_and_is_not_resent_after_restart() {
    let first_scratch = Scratch::new();
    let response =
        ExpectedRequest::accepted_post_then_lost_response("post", post(), "1791271000.000900")
            .unwrap()
            .held();
    let (mut first, first_fixture) =
        at_reply_confirmation(Mode::Write, vec![response], &first_scratch).await;
    key(&mut first, Key::Char('y'));
    first
        .settle_until(BUDGET, |_| writes(&first_fixture) == 1)
        .await
        .unwrap();
    pending_reply(&first, &first_fixture);
    let receipt = accepted_pending(&first_fixture);
    first_fixture.release("post").unwrap();
    first.settle(BUDGET).await.unwrap();
    assert!(
        first
            .host()
            .message
            .as_deref()
            .unwrap()
            .contains("reply outcome uncertain; a post may already exist")
    );
    assert!(
        !first.records().iter().any(|record| matches!(record,
        JournalRecord::Begin { descriptor, .. }
            if descriptor.kind.as_str() == "slack.message.post.result")),
        "a lost response must not invent a successful post result"
    );
    reopen_retained_reply(&mut first);
    first.advance_time(NOW + 1).unwrap();
    first.settle(BUDGET).await.unwrap();
    assert_eq!(writes(&first_fixture), 1);
    assert_eq!(first_fixture.accepted_posts(), vec![receipt.clone()]);
    let cache = CapturedSlackCache::capture(&first);
    assert!(
        !cache.value().to_string().contains(BODY),
        "kept drafts are session-local, not serialized cache data"
    );
    first_fixture.assert_finished();
    journal(&mut first, &first_scratch, Outcome::Unknown);
    let prior_events = serde_json::to_value(persisted_events(&first_scratch)).unwrap();
    drop(first);

    // A fresh in-process Host with a separate scratch journal. This exercises
    // serialized snapshot/restore, not cache SQLite or process/same-DB recovery.
    let second_scratch = Scratch::new();
    let (mut second, second_fixture) = at_restored_cached_feed(&cache, &receipt, &second_scratch);
    assert!(second_fixture.ledger().is_empty());
    second.advance_time(NOW + 1).unwrap();
    second
        .settle_until(BUDGET, |_| second_fixture.ledger().len() == 1)
        .await
        .unwrap();
    activate_gate_thread(&mut second);
    restored_before_auth(&second, &second_fixture);
    second_fixture.release("identity").unwrap();
    second.settle(BUDGET).await.unwrap();
    assert_eq!(
        second_fixture.ledger().len(),
        5,
        "fresh auth and real feed reads"
    );
    assert!(
        !second.host().plugins[0]
            .top_right("feed")
            .unwrap()
            .starts_with("cached")
    );
    activate_gate_thread(&mut second);
    second.settle(BUDGET).await.unwrap();
    second.advance_time(NOW + 2).unwrap();
    second.settle(BUDGET).await.unwrap();
    restarted_thread(&second, &second_fixture, &receipt);
    second_fixture.assert_finished();
    second.finish_journal().unwrap();

    struct SeparateHistories {
        prior_unchanged: bool,
        fresh: Vec<ActionEvent>,
    }
    let checkpoint = SeparateHistories {
        prior_unchanged: serde_json::to_value(persisted_events(&first_scratch)).unwrap()
            == prior_events,
        fresh: persisted_events(&second_scratch),
    };
    verify(
        "independent fresh-session history",
        &checkpoint,
        &[
            Probe {
                name: "original unknown fact retained",
                check: |c| {
                    require(
                        c.prior_unchanged,
                        "later provider reads changed the prior Unknown history",
                    )
                },
            },
            Probe {
                name: "new session records no post action",
                check: |c| {
                    require(
                        c.fresh.iter().all(|e| {
                            !matches!(
                                e.kind.as_str(),
                                "slack.reply.post" | "slack.message.post.result"
                            )
                        }),
                        "new session invented a post action or success result",
                    )
                },
            },
            Probe {
                name: "new session records successful thread read",
                check: |c| {
                    require(
                        c.fresh.iter().any(|e| {
                            e.kind.as_str() == "slack.page.thread"
                                && e.outcome == Outcome::Succeeded
                        }),
                        "new session did not persist its real provider read",
                    )
                },
            },
            Probe {
                name: "message text excluded",
                check: |c| {
                    require(
                        !serde_json::to_string(&c.fresh).unwrap().contains(BODY),
                        "new history contains message text",
                    )
                },
            },
        ],
    )
    .unwrap_or_else(|error| panic!("{error}"));
}
