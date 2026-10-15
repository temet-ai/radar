use super::*;
use crate::client::{Recorded, Reply, Request};
use anyhow::anyhow;
use serde_json::json;
use std::sync::{Arc, Mutex};

type Waits = Arc<Mutex<Vec<u64>>>;

fn ok(body: Value) -> Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Reply::default()
    })
}

fn limited(seconds: u64) -> Result<Reply> {
    Ok(Reply {
        status: 429,
        retry_after: Some(seconds),
        ..Reply::default()
    })
}

fn setup(replies: Vec<Result<Reply>>) -> (Client, Arc<Recorded>, Waits) {
    let recorded = Recorded::new(replies);
    let waits = Arc::new(Mutex::new(Vec::new()));
    let log = waits.clone();
    let client = Client::new(recorded.clone()).with_wait(Arc::new(move |seconds| {
        log.lock().unwrap().push(seconds);
        Box::pin(async {})
    }));
    (client, recorded, waits)
}

fn channel(id: &str, name: &str) -> Value {
    json!({
        "id": id, "name": name, "is_private": false, "is_member": false,
        "is_im": false, "is_archived": false,
        "topic": {"value": "planning"}, "purpose": {"value": "the team"}
    })
}

fn page(rows: Vec<Value>, next: &str) -> Result<Reply> {
    ok(json!({"ok": true, "channels": rows, "response_metadata": {"next_cursor": next}}))
}

fn list_request(kind: ChannelKind, cursor: &str) -> Request {
    let (method, types, limit) = match kind {
        ChannelKind::Yours => (
            "users.conversations",
            "public_channel,private_channel",
            "999",
        ),
        ChannelKind::Public => ("conversations.list", "public_channel", "1000"),
        ChannelKind::Info => panic!("not a list"),
    };
    let mut params = vec![
        ("types".into(), types.into()),
        ("exclude_archived".into(), "true".into()),
        ("limit".into(), limit.into()),
    ];
    if !cursor.is_empty() {
        params.push(("cursor".into(), cursor.into()));
    }
    Client::read(method, params)
}

#[tokio::test]
async fn observed_info_preserves_all_values_request_and_retry_behavior() {
    let row = json!({
        "id": "C1", "name": " Team ", "is_private": true, "is_member": false,
        "is_im": true, "is_archived": true,
        "topic": {"value": " topic "}, "purpose": {"value": " purpose "}
    });
    let (client, recorded, waits) =
        setup(vec![limited(7), ok(json!({"ok": true, "channel": row}))]);
    let got = client.channel_info_observed("C1").await;
    assert_eq!(got.result.unwrap(), channel_of(&row));
    let mut expected = ChannelPhase::new(ChannelKind::Info);
    expected.requests = 1;
    expected.returned = 1;
    expected.end = ChannelEnd::Found;
    expected.membership = Some(ChannelMembership::Explicit);
    assert_eq!(got.evidence.phases, [Some(expected), None]);
    assert!(!got.evidence.has_gaps());
    assert_eq!(got.evidence.end(), Some(ChannelEnd::Found));
    let request = Client::read("conversations.info", vec![("channel".into(), "C1".into())]);
    assert_eq!(recorded.requests(), [request.clone(), request]);
    assert_eq!(*waits.lock().unwrap(), [7]);

    let (legacy, _, _) = setup(vec![ok(json!({"ok": true, "channel": row}))]);
    assert_eq!(legacy.channel_info("C1").await.unwrap(), channel_of(&row));
}

#[tokio::test]
async fn observed_info_records_missing_malformed_and_identity_gaps_without_rejecting_values() {
    let mut mismatched = channel("C2", "team");
    mismatched["is_member"] = json!(true);
    let mut blank = channel(" ", " \t ");
    blank["is_member"] = json!(false);
    let malformed = json!({
        "id": 7, "name": false, "is_private": "true", "is_member": 0,
        "is_im": null, "is_archived": [], "topic": {"value": 1}, "purpose": []
    });
    for (row, defaults, gap, incomplete, member) in [
        (None, 8, 1, 1, ChannelMembership::Defaulted),
        (Some(Value::Null), 8, 1, 1, ChannelMembership::Defaulted),
        (Some(json!([])), 8, 1, 1, ChannelMembership::Defaulted),
        (Some(json!({})), 8, 1, 0, ChannelMembership::Defaulted),
        (Some(malformed), 8, 1, 0, ChannelMembership::Defaulted),
        (Some(mismatched), 0, 1, 0, ChannelMembership::Explicit),
        (Some(blank), 0, 1, 0, ChannelMembership::Explicit),
        (
            Some(channel("C1", "team")),
            0,
            0,
            0,
            ChannelMembership::Explicit,
        ),
    ] {
        let mut body = json!({"ok": true});
        if let Some(row) = &row {
            body["channel"] = row.clone();
        }
        let (client, recorded, _) = setup(vec![ok(body)]);
        let got = client.channel_info_observed("C1").await;
        assert_eq!(
            got.result.unwrap(),
            channel_of(row.as_ref().unwrap_or(&Value::Null))
        );
        let evidence = got.evidence.phases[0].unwrap();
        assert_eq!(evidence.requests, 1);
        assert_eq!(evidence.returned, usize::from(row.is_some()));
        assert_eq!(evidence.defaulted_fields, defaults);
        assert_eq!(evidence.identity_gaps, gap);
        assert_eq!(evidence.incomplete_pages, incomplete);
        assert_eq!(evidence.membership, Some(member));
        assert_eq!(evidence.end, ChannelEnd::Found);
        assert_eq!(got.evidence.has_gaps(), defaults + gap + incomplete > 0);
        assert_eq!(recorded.requests().len(), 1);
    }
}

#[tokio::test]
async fn observed_info_request_failures_keep_raw_errors_and_zero_completed_counts() {
    let cases = [
        (
            Err(anyhow!("provider-marker")),
            "could not reach Slack: provider-marker",
        ),
        (
            ok(json!({"ok": false, "error": "channel_not_found"})),
            "no such channel, or the token cannot see it",
        ),
    ];
    for (reply, expected) in cases {
        let (client, recorded, _) = setup(vec![reply]);
        let got = client.channel_info_observed("C1").await;
        assert_eq!(got.result.unwrap_err().to_string(), expected);
        assert_eq!(
            got.evidence.phases,
            [Some(ChannelPhase::new(ChannelKind::Info)), None]
        );
        assert_eq!(got.evidence.end(), Some(ChannelEnd::Request));
        assert_eq!(recorded.requests().len(), 1);
    }
    let (client, recorded, waits) = setup((0..4).map(|_| limited(1)).collect());
    let got = client.channel_info_observed("C1").await;
    assert_eq!(
        got.result.unwrap_err().to_string(),
        "Slack is still rate limiting after 3 tries"
    );
    assert_eq!(got.evidence.phases[0].unwrap().requests, 0);
    assert_eq!(recorded.requests().len(), 4);
    assert_eq!(*waits.lock().unwrap(), [1, 1, 1]);
}

fn down(retry_after: Option<u64>) -> Result<Reply> {
    Ok(Reply {
        status: 503,
        retry_after,
        ..Reply::default()
    })
}

#[tokio::test]
async fn a_read_slack_cannot_answer_now_waits_longer_each_time_then_says_so() {
    let (client, recorded, waits) = setup((0..4).map(|_| down(None)).collect());
    let got = client.channel_info_observed("C1").await;
    assert_eq!(got.result.unwrap_err().to_string(), "Slack answered HTTP 503");
    assert_eq!(recorded.requests().len(), 4);
    assert_eq!(*waits.lock().unwrap(), [1, 2, 4]);

    let (client, recorded, waits) = setup(vec![
        down(Some(7)),
        ok(json!({"ok": true, "channel": channel("C1", "team")})),
    ]);
    let got = client.channel_info_observed("C1").await;
    assert_eq!(got.result.unwrap().name, "team");
    assert_eq!(recorded.requests().len(), 2);
    assert_eq!(*waits.lock().unwrap(), [7], "Slack's own Retry-After");
}

#[tokio::test]
async fn a_write_slack_cannot_answer_now_is_never_sent_twice() {
    let (client, recorded, waits) = setup(vec![down(None)]);
    let err = client.post("C1", "hi", None).await.unwrap_err();
    assert_eq!(err.to_string(), "Slack answered HTTP 503");
    assert_eq!(recorded.requests().len(), 1);
    assert!(waits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn observed_yours_counts_every_matched_page_row_and_duplicate_after_retry() {
    let mut target = channel("C1", "TeAm");
    target.as_object_mut().unwrap().remove("is_member");
    let (client, recorded, waits) = setup(vec![
        page(vec![channel("C0", "other")], "next"),
        limited(4),
        page(vec![target.clone(), target.clone(), Value::Null], "unread"),
        page(vec![], ""),
    ]);
    let got = client.find_channel_observed("  #TEAM  ").await;
    let result = got.result.unwrap();
    let mut parsed = channel_of(&target);
    parsed.is_member = true;
    assert_eq!(
        result,
        Lookup {
            found: Some(parsed),
            yours: 4,
            public: 0
        }
    );
    let evidence = got.evidence.phases[0].unwrap();
    assert_eq!(evidence.kind, ChannelKind::Yours);
    assert_eq!((evidence.requests, evidence.returned), (2, 4));
    // The matched row's member flag is list-derived. The duplicate's missing
    // member flag and all eight fields in the trailing null still default.
    assert_eq!(evidence.defaulted_fields, 9);
    assert_eq!((evidence.identity_gaps, evidence.incomplete_pages), (1, 0));
    assert_eq!(evidence.membership, Some(ChannelMembership::ListDerived));
    assert_eq!(evidence.end, ChannelEnd::Found);
    assert!(got.evidence.phases[1].is_none());
    assert_eq!(
        recorded.requests(),
        [
            list_request(ChannelKind::Yours, ""),
            list_request(ChannelKind::Yours, "next"),
            list_request(ChannelKind::Yours, "next"),
        ]
    );
    assert_eq!(*waits.lock().unwrap(), [4]);
}

#[tokio::test]
async fn observed_membership_sources_preserve_yours_override_and_public_parser_flags() {
    for field in [
        None,
        Some(json!(true)),
        Some(json!(false)),
        Some(json!("true")),
    ] {
        let mut row = channel("C1", "team");
        row.as_object_mut().unwrap().remove("is_member");
        if let Some(field) = &field {
            row["is_member"] = field.clone();
        }
        for kind in [ChannelKind::Yours, ChannelKind::Public] {
            let replies = if kind == ChannelKind::Yours {
                vec![page(vec![row.clone()], "unread")]
            } else {
                vec![page(vec![], ""), page(vec![row.clone()], "unread")]
            };
            let (client, recorded, _) = setup(replies);
            let got = client.find_channel_observed("team").await;
            let result = got.result.unwrap();
            let phase = got.evidence.phases[usize::from(kind == ChannelKind::Public)].unwrap();
            let explicit = field.as_ref().and_then(Value::as_bool);
            let expected_member = kind == ChannelKind::Yours || explicit.unwrap_or(false);
            assert_eq!(result.found.unwrap().is_member, expected_member);
            assert_eq!(
                phase.defaulted_fields,
                usize::from(kind == ChannelKind::Public && explicit.is_none())
            );
            assert_eq!(
                phase.membership,
                Some(if kind == ChannelKind::Yours {
                    ChannelMembership::ListDerived
                } else if explicit.is_some() {
                    ChannelMembership::Explicit
                } else {
                    ChannelMembership::Defaulted
                })
            );
            assert_eq!(phase.end, ChannelEnd::Found);
            let expected = if kind == ChannelKind::Yours {
                vec![list_request(kind, "")]
            } else {
                vec![list_request(ChannelKind::Yours, ""), list_request(kind, "")]
            };
            assert_eq!(recorded.requests(), expected);
        }
    }
}

#[tokio::test]
async fn observed_public_fallback_keeps_completed_phases_counts_and_exact_params() {
    let (client, recorded, _) = setup(vec![
        page(vec![channel("C0", "random")], ""),
        page(
            vec![channel("C2", "general"), channel("C2", "general")],
            "public-next",
        ),
        page(
            vec![channel("C4", "deploys"), channel("C5", "trailing")],
            "unused",
        ),
    ]);
    let got = client.find_channel_observed("deploys").await;
    let result = got.result.unwrap();
    assert_eq!((result.yours, result.public), (1, 4));
    assert_eq!(result.found.unwrap(), channel_of(&channel("C4", "deploys")));
    let yours = got.evidence.phases[0].unwrap();
    assert_eq!(
        (yours.requests, yours.returned, yours.end),
        (1, 1, ChannelEnd::NoMatch)
    );
    assert!(yours.membership.is_none());
    let public = got.evidence.phases[1].unwrap();
    assert_eq!(
        (public.requests, public.returned, public.end),
        (2, 4, ChannelEnd::Found)
    );
    assert!(!got.evidence.has_gaps());
    assert_eq!(
        recorded.requests(),
        [
            list_request(ChannelKind::Yours, ""),
            list_request(ChannelKind::Public, ""),
            list_request(ChannelKind::Public, "public-next"),
        ]
    );
}

#[tokio::test]
async fn observed_lookup_distinguishes_no_match_from_request_and_local_cap() {
    let (client, recorded, _) = setup(vec![
        page(vec![channel("C0", "random")], ""),
        page(vec![channel("C2", "general")], ""),
    ]);
    let got = client.find_channel_observed("team").await;
    assert_eq!(
        got.result.unwrap(),
        Lookup {
            found: None,
            yours: 1,
            public: 1
        }
    );
    assert_eq!(got.evidence.end(), Some(ChannelEnd::NoMatch));
    assert!(
        got.evidence
            .phases
            .into_iter()
            .flatten()
            .all(|p| p.end == ChannelEnd::NoMatch)
    );
    assert!(!got.evidence.has_gaps());
    assert_eq!(recorded.requests().len(), 2);
    assert_eq!(ChannelEvidence::default().end(), None);
}

#[tokio::test]
async fn observed_list_missing_or_malformed_collection_and_cursor_do_not_change_traversal() {
    for body in [
        json!({"ok": true, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": {}, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": null, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": []}),
        json!({"ok": true, "channels": [], "response_metadata": null}),
        json!({"ok": true, "channels": [], "response_metadata": {}}),
        json!({"ok": true, "channels": [], "response_metadata": {"next_cursor": 8}}),
        json!({"ok": true, "channels": [], "response_metadata": {"next_cursor": null}}),
    ] {
        let (client, recorded, _) = setup(vec![ok(body.clone()), page(vec![], "")]);
        let got = client.find_channel_observed("team").await;
        assert_eq!(got.result.unwrap(), Lookup::default(), "{body}");
        let first = got.evidence.phases[0].unwrap();
        assert_eq!(
            (
                first.requests,
                first.returned,
                first.defaulted_fields,
                first.identity_gaps
            ),
            (1, 0, 0, 0)
        );
        assert_eq!(first.incomplete_pages, 1, "{body}");
        assert_eq!(first.end, ChannelEnd::NoMatch);
        assert_eq!(got.evidence.phases[1].unwrap().incomplete_pages, 0);
        assert_eq!(recorded.requests().len(), 2);
    }
}

#[tokio::test]
async fn observed_whitespace_cursor_is_recorded_as_gap_but_sent_unchanged() {
    let (client, recorded, _) = setup(vec![
        page(vec![], " \t "),
        page(vec![channel("C1", "team")], ""),
    ]);
    let got = client.find_channel_observed("team").await;
    assert_eq!(got.result.unwrap().found.unwrap().id, "C1");
    let phase = got.evidence.phases[0].unwrap();
    assert_eq!(
        (phase.requests, phase.returned, phase.incomplete_pages),
        (2, 1, 1)
    );
    assert_eq!(phase.end, ChannelEnd::Found);
    assert_eq!(
        recorded.requests(),
        [
            list_request(ChannelKind::Yours, ""),
            list_request(ChannelKind::Yours, " \t "),
        ]
    );
}

#[tokio::test]
async fn observed_list_counts_malformed_rows_and_blank_identity_including_after_match() {
    let mut blank_id = channel(" ", "team");
    blank_id["topic"] = json!({"value": false});
    let blank_name = channel("C2", " ");
    let (client, recorded, _) = setup(vec![ok(json!({"ok": true,
        "channels": [blank_id.clone(), blank_name, null, false]}))]);
    let got = client.find_channel_observed("team").await;
    let mut expected = channel_of(&blank_id);
    expected.is_member = true;
    assert_eq!(
        got.result.unwrap(),
        Lookup {
            found: Some(expected),
            yours: 4,
            public: 0
        }
    );
    let phase = got.evidence.phases[0].unwrap();
    assert_eq!(phase.returned, 4);
    assert_eq!(phase.defaulted_fields, 17);
    assert_eq!(phase.identity_gaps, 4);
    assert_eq!(phase.incomplete_pages, 1);
    assert_eq!(phase.end, ChannelEnd::Found);
    assert_eq!(recorded.requests().len(), 1);
}

#[tokio::test]
async fn observed_page_one_hundred_match_wins_over_cursor_and_cap_for_either_phase() {
    for kind in [ChannelKind::Yours, ChannelKind::Public] {
        for last_cursor in ["", "one-more"] {
            let mut replies = Vec::new();
            if kind == ChannelKind::Public {
                replies.push(page(vec![], ""));
            }
            for index in 0..MAX_LIST_PAGES - 1 {
                replies.push(page(vec![channel("C0", "other")], &format!("next-{index}")));
            }
            replies.push(page(
                vec![channel("C1", "team"), channel("C1", "team")],
                last_cursor,
            ));
            let (client, recorded, _) = setup(replies);
            let got = client.find_channel_observed("team").await;
            assert_eq!(got.result.unwrap().found.unwrap().id, "C1");
            let phase = got.evidence.phases[usize::from(kind == ChannelKind::Public)].unwrap();
            assert_eq!(
                (phase.requests, phase.returned, phase.end),
                (100, 101, ChannelEnd::Found)
            );
            assert_eq!(
                recorded.requests().len(),
                100 + usize::from(kind == ChannelKind::Public)
            );
            assert!(!got.evidence.has_gaps());
        }
    }
}

#[tokio::test]
async fn observed_page_one_hundred_explicit_end_is_no_match_not_cap() {
    for kind in [ChannelKind::Yours, ChannelKind::Public] {
        let mut replies = Vec::new();
        if kind == ChannelKind::Public {
            replies.push(page(vec![], ""));
        }
        for index in 0..MAX_LIST_PAGES - 1 {
            replies.push(page(vec![], &format!("next-{index}")));
        }
        replies.push(page(vec![], ""));
        if kind == ChannelKind::Yours {
            replies.push(page(vec![], ""));
        }
        let (client, recorded, _) = setup(replies);
        let got = client.find_channel_observed("absent").await;
        assert_eq!(got.result.unwrap(), Lookup::default());
        let phase = got.evidence.phases[usize::from(kind == ChannelKind::Public)].unwrap();
        assert_eq!(
            (phase.requests, phase.returned, phase.end),
            (100, 0, ChannelEnd::NoMatch)
        );
        assert_eq!(got.evidence.end(), Some(ChannelEnd::NoMatch));
        assert_eq!(recorded.requests().len(), 101);
    }
}

#[tokio::test]
async fn observed_local_cap_retains_counts_and_does_not_fall_back_from_yours() {
    for kind in [ChannelKind::Yours, ChannelKind::Public] {
        let mut replies = Vec::new();
        if kind == ChannelKind::Public {
            replies.push(page(vec![channel("C0", "other")], ""));
        }
        for index in 0..MAX_LIST_PAGES {
            replies.push(page(vec![channel("C0", "other")], &format!("next-{index}")));
        }
        // Must remain unread, including when the first phase hits its cap.
        replies.push(page(vec![channel("C1", "team")], ""));
        let (client, recorded, _) = setup(replies);
        let got = client.find_channel_observed("team").await;
        assert_eq!(
            got.result.unwrap_err().to_string(),
            "Slack listed more than 100 pages of channels; paste a link to it instead"
        );
        let phase = got.evidence.phases[usize::from(kind == ChannelKind::Public)].unwrap();
        assert_eq!(
            (phase.requests, phase.returned, phase.end),
            (100, 100, ChannelEnd::LocalCap)
        );
        assert_eq!(got.evidence.end(), Some(ChannelEnd::LocalCap));
        assert!(phase.membership.is_none());
        if kind == ChannelKind::Yours {
            assert!(got.evidence.phases[1].is_none());
        } else {
            assert_eq!(got.evidence.phases[0].unwrap().end, ChannelEnd::NoMatch);
        }
        assert_eq!(
            recorded.requests().len(),
            100 + usize::from(kind == ChannelKind::Public)
        );
        assert_eq!(recorded.replies.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn observed_request_failure_keeps_completed_pages_and_phases_without_fallback() {
    for kind in [ChannelKind::Yours, ChannelKind::Public] {
        for completed in [0, 2] {
            let mut replies = Vec::new();
            if kind == ChannelKind::Public {
                replies.push(page(vec![channel("C0", "other")], ""));
            }
            for index in 0..completed {
                replies.push(page(
                    vec![channel("C2", "other"), Value::Null],
                    &format!("next-{index}"),
                ));
            }
            replies.push(Err(anyhow!("raw-provider-marker")));
            replies.push(page(vec![channel("C1", "team")], ""));
            let (client, recorded, _) = setup(replies);
            let got = client.find_channel_observed("team").await;
            assert_eq!(
                got.result.unwrap_err().to_string(),
                "could not reach Slack: raw-provider-marker"
            );
            let phase = got.evidence.phases[usize::from(kind == ChannelKind::Public)].unwrap();
            assert_eq!(
                (phase.requests, phase.returned, phase.end),
                (completed, 2 * completed, ChannelEnd::Request)
            );
            assert_eq!(phase.defaulted_fields, 8 * completed);
            assert_eq!(phase.identity_gaps, completed);
            assert_eq!(got.evidence.end(), Some(ChannelEnd::Request));
            assert!(phase.membership.is_none());
            if kind == ChannelKind::Yours {
                assert!(got.evidence.phases[1].is_none());
            } else {
                assert_eq!(got.evidence.phases[0].unwrap().end, ChannelEnd::NoMatch);
            }
            assert_eq!(
                recorded.requests().len(),
                completed + 1 + usize::from(kind == ChannelKind::Public)
            );
            assert_eq!(recorded.replies.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn observed_empty_name_matching_remains_permissive_and_marks_identity_gaps() {
    let (client, recorded, _) = setup(vec![page(vec![Value::Null, Value::Null], "unread")]);
    let got = client.find_channel_observed(" # ").await;
    assert_eq!(
        got.result.unwrap(),
        Lookup {
            found: Some(ChannelInfo {
                is_member: true,
                ..ChannelInfo::default()
            }),
            yours: 2,
            public: 0,
        }
    );
    let phase = got.evidence.phases[0].unwrap();
    assert_eq!(
        (phase.returned, phase.defaulted_fields, phase.identity_gaps),
        (2, 15, 2)
    );
    assert_eq!(phase.membership, Some(ChannelMembership::ListDerived));
    assert_eq!(phase.end, ChannelEnd::Found);
    assert_eq!(recorded.requests().len(), 1);
}
