//! Name-lookup feedback follows typed evidence, without inferring permissions
//! from cached account data. All requests below use recordings.

use super::*;
use crate::client::Request;

fn failed_text(p: &SlackPlugin) -> &str {
    let Check::Failed(text) = &p.checks["add"].0 else {
        panic!("negative lookup feedback");
    };
    text
}

fn incomplete(text: &str) {
    assert!(
        text.contains("Search for #PRIVATE_NAME was incomplete"),
        "{text}"
    );
    assert!(text.contains("a match could not be established"), "{text}");
    assert!(text.contains("channel ID or link"), "{text}");
    for claim in [
        "no channel called",
        "not among",
        "cannot read private",
        "groups:read",
    ] {
        assert!(!text.contains(claim), "{text}");
    }
}

fn list_request(public: bool, cursor: &str) -> Request {
    let mut params = vec![
        (
            "types".into(),
            if public {
                "public_channel"
            } else {
                "public_channel,private_channel"
            }
            .into(),
        ),
        ("exclude_archived".into(), "true".into()),
        ("limit".into(), if public { "1000" } else { "999" }.into()),
    ];
    if !cursor.is_empty() {
        params.push(("cursor".into(), cursor.into()));
    }
    Request {
        method: if public {
            "conversations.list"
        } else {
            "users.conversations"
        },
        params,
        body: None,
        write: false,
    }
}

#[test]
fn complete_negative_reports_returned_occurrences_independent_of_cached_scopes() {
    for empty in [false, true] {
        let mut expected_text = None;
        for scopes in [vec![], vec!["channels:read"], vec!["groups:read"]] {
            let rows = if empty {
                vec![]
            } else {
                vec![row(ID, "other"), row(ID, "other")]
            };
            let (mut p, rec) = sample(vec![page(rows.clone(), ""), page(rows, "")]);
            p.data.me.as_mut().unwrap().scopes = scopes.into_iter().map(str::to_string).collect();
            let before = (p.cfg.clone(), p.data.clone(), p.snapshot());
            let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
            let fx = p.on_msg(run(task));
            report(&fx, Outcome::Failed);
            let n = if empty { 0 } else { 2 };
            observation(&fx, "yours.returned", Outcome::Succeeded, n);
            observation(&fx, "public.returned", Outcome::Succeeded, n);
            observation(&fx, "no_match", Outcome::Failed, 1);
            let text = failed_text(&p);
            assert!(
                text.starts_with("No match for #PRIVATE_NAME in the returned lists:"),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "{n} joined-channel entries and {n} public-channel entries"
                )),
                "{text}"
            );
            assert!(text.contains("duplicates included"), "{text}");
            assert!(text.contains("channel ID or link"), "{text}");
            for claim in [
                "incomplete",
                "you can see",
                "you are in",
                "groups:read",
                "cannot read private",
                "workspace",
            ] {
                assert!(!text.contains(claim), "{text}");
            }
            assert_eq!(text, expected_text.get_or_insert_with(|| text.to_string()));
            assert_eq!((p.cfg.clone(), p.data.clone(), p.snapshot()), before);
            assert_eq!(
                rec.requests(),
                [list_request(false, ""), list_request(true, "")]
            );
        }
    }
}

#[test]
fn incomplete_name_search_never_claims_absence_or_cached_permission_failure() {
    let mut defaulted = row(ID, "other");
    defaulted.as_object_mut().unwrap().remove("topic");
    let bodies = [
        json!({"ok": true}),
        json!({"ok": true, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": {}, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": null, "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": []}),
        json!({"ok": true, "channels": [], "response_metadata": {"next_cursor": 8}}),
        json!({"ok": true, "channels": [defaulted], "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": [row(" ", "other")], "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": [row(ID, " ")], "response_metadata": {"next_cursor": ""}}),
        json!({"ok": true, "channels": [null], "response_metadata": {"next_cursor": ""}}),
    ];
    for body in bodies {
        for public in [false, true] {
            for private_scope in [false, true] {
                let replies = if public {
                    vec![page(vec![], ""), ok(body.clone())]
                } else {
                    vec![ok(body.clone()), page(vec![], "")]
                };
                let (mut p, rec) = sample(replies);
                p.data.me.as_mut().unwrap().scopes = if private_scope {
                    vec!["groups:read".into()]
                } else {
                    vec![]
                };
                let before = (p.cfg.clone(), p.data.clone(), p.snapshot());
                let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
                let fx = p.on_msg(run(task));
                report(&fx, Outcome::Partial);
                observation(&fx, "no_match.incomplete", Outcome::Partial, 1);
                let phase = if public { "public" } else { "yours" };
                observation(
                    &fx,
                    &format!("{phase}.no_match.incomplete"),
                    Outcome::Partial,
                    1,
                );
                incomplete(failed_text(&p));
                assert!(!failed_text(&p).contains("page limit"));
                assert!(!format!("{:?}", history(&fx)).contains("request.failed"));
                assert_eq!((p.cfg.clone(), p.data.clone(), p.snapshot()), before);
                assert_eq!(
                    rec.requests(),
                    [list_request(false, ""), list_request(true, "")]
                );
            }
        }
    }
}

#[test]
fn whitespace_cursor_and_retry_keep_actual_requests_and_partial_counts() {
    let (mut p, rec) = sample(vec![
        Ok(Reply {
            status: 429,
            retry_after: Some(0),
            ..Default::default()
        }),
        page(vec![row(ID, "other")], " \t "),
        page(vec![row(ID, "other")], ""),
        page(vec![], ""),
    ]);
    let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
    let fx = p.on_msg(run(task));
    report(&fx, Outcome::Partial);
    observation(&fx, "yours.requests", Outcome::Succeeded, 2);
    observation(&fx, "yours.returned", Outcome::Partial, 2);
    observation(&fx, "yours.pages.incomplete", Outcome::Partial, 1);
    observation(&fx, "public.requests", Outcome::Succeeded, 1);
    incomplete(failed_text(&p));
    assert_eq!(
        rec.requests(),
        [
            list_request(false, ""),
            list_request(false, ""),
            list_request(false, " \t "),
            list_request(true, ""),
        ]
    );
}

#[test]
fn local_page_cap_is_incomplete_feedback_without_a_failed_request() {
    for public in [false, true] {
        let mut replies = Vec::new();
        if public {
            replies.push(page(vec![], ""));
        }
        replies.extend((0..100).map(|_| page(vec![row(ID, "other")], "PRIVATE_CURSOR")));
        let (mut p, rec) = sample(replies);
        let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
        let fx = p.on_msg(run(task));
        report(&fx, Outcome::Partial);
        let phase = if public { "public" } else { "yours" };
        observation(&fx, &format!("{phase}.requests"), Outcome::Succeeded, 100);
        observation(&fx, &format!("{phase}.returned"), Outcome::Succeeded, 100);
        assert!(
            observation(&fx, &format!("{phase}.cap"), Outcome::Partial, 1)
                .metadata
                .truncated
        );
        incomplete(failed_text(&p));
        assert!(failed_text(&p).contains("page limit"));
        assert!(!format!("{:?}", history(&fx)).contains("request.failed"));
        let mut expected = Vec::new();
        if public {
            expected.push(list_request(false, ""));
        }
        expected.push(list_request(public, ""));
        expected.extend((1..100).map(|_| list_request(public, "PRIVATE_CURSOR")));
        assert_eq!(rec.requests(), expected);
    }
}

#[test]
fn request_errors_keep_actual_diagnosis_even_when_text_resembles_local_cap() {
    for (reply, expected) in [
        (
            Err(anyhow::anyhow!(
                "Slack listed more than 100 pages of channels; paste a link to it instead"
            )),
            "could not reach Slack: Slack listed more than 100 pages of channels; paste a link to it instead",
        ),
        (
            ok(json!({"ok": false, "error": "missing_scope", "needed": "groups:read"})),
            "the token lacks a permission: groups:read",
        ),
    ] {
        let (mut p, rec) = sample(vec![page(vec![row(ID, "other")], ""), reply]);
        p.data.me.as_mut().unwrap().scopes = vec!["groups:read".into()];
        let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
        let fx = p.on_msg(run(task));
        report(&fx, Outcome::Failed);
        observation(&fx, "yours.requests", Outcome::Succeeded, 1);
        observation(&fx, "yours.returned", Outcome::Succeeded, 1);
        observation(&fx, "public.request.failed", Outcome::Failed, 1);
        assert_eq!(failed_text(&p), expected);
        assert!(!format!("{:?}", history(&fx)).contains("resolve.no_match"));
        assert_eq!(
            rec.requests(),
            [list_request(false, ""), list_request(true, "")]
        );
    }
}

fn dispatch_name(host: &mut Host) -> (ActionId, ActionId, ActionId, Task) {
    let fx = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Command::Name),
    });
    let begun = records(&fx);
    private_absent(&format!("{begun:?}"));
    let parent = id(&begun, "test.enclosing");
    let lookup = id(&begun, PREFIX);
    assert_eq!(terminal(&begun, &parent), [Outcome::Succeeded]);
    assert!(terminal(&begun, &lookup).is_empty());
    let (runtime, semantic, task) = fx
        .into_iter()
        .find_map(|fx| match fx {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(semantic, Some(lookup.clone()));
    (parent, lookup, runtime, task)
}

fn deliver(
    host: &mut Host,
    lookup: &ActionId,
    runtime: &ActionId,
    task: Task,
) -> Vec<JournalRecord> {
    let fx = host.update(Event::Tracked {
        action: runtime.clone(),
        semantic: Some(lookup.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: run(task),
        }),
    });
    assert!(
        fx.iter().all(|fx| matches!(fx, HostEffect::Journal(_))),
        "no save, cache write or follow-up task"
    );
    let done = records(&fx);
    private_absent(&format!("{done:?}"));
    done
}

#[test]
fn host_negative_feedback_preserves_original_outcomes_without_durable_payloads() {
    for gap in [false, true] {
        let first = if gap {
            ok(json!({"ok": true}))
        } else {
            page(vec![], "")
        };
        let (p, rec) = sample(vec![first, page(vec![], "")]);
        let shared = Arc::new(Mutex::new(p));
        let mut host = Host::new(vec![Box::new(ChannelHost(shared.clone()))], demo::NOW).unwrap();
        host.enable_journal();
        let before = {
            let p = shared.lock().unwrap();
            (p.config(), p.snapshot())
        };
        let (parent, lookup, runtime, task) = dispatch_name(&mut host);
        assert!(rec.requests().is_empty());
        let done = deliver(&mut host, &lookup, &runtime, task);
        let expected = if gap {
            Outcome::Partial
        } else {
            Outcome::Failed
        };
        assert_eq!(terminal(&done, &lookup), [expected]);
        assert_eq!(terminal(&done, &runtime), [expected]);
        assert!(terminal(&done, &parent).is_empty());
        let p = shared.lock().unwrap();
        if gap {
            incomplete(failed_text(&p));
        } else {
            assert!(failed_text(&p).starts_with("No match for"));
        }
        assert_eq!(
            host.message.as_ref().unwrap().text,
            format!("✗ {}", failed_text(&p))
        );
        assert_eq!((p.config(), p.snapshot()), before);
        assert_eq!(
            rec.requests(),
            [list_request(false, ""), list_request(true, "")]
        );
    }
}

#[test]
fn host_late_incomplete_negative_cannot_replace_newer_feedback_after_token_change() {
    let (p, rec) = sample(vec![ok(json!({"ok": true})), page(vec![], "")]);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(ChannelHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let (_, old_lookup, old_runtime, old_task) = dispatch_name(&mut host);
    let replacement = Recorded::new(vec![page(vec![], ""), page(vec![], "")]);
    {
        let mut p = shared.lock().unwrap();
        let me = p.data.me.clone().unwrap();
        p.replace_token("env:PRIVATE_REPLACEMENT".into());
        assert!(p.install_me(me));
        p.via = Via(Some(replacement.clone()));
    }
    let (_, new_lookup, new_runtime, new_task) = dispatch_name(&mut host);
    let done = deliver(&mut host, &new_lookup, &new_runtime, new_task);
    assert_eq!(terminal(&done, &new_lookup), [Outcome::Failed]);
    let before = {
        let p = shared.lock().unwrap();
        (p.cfg.clone(), p.data.clone(), p.checks.clone())
    };
    let notice = host.message.as_ref().unwrap().text.clone();
    let done = deliver(&mut host, &old_lookup, &old_runtime, old_task);
    assert_eq!(terminal(&done, &old_lookup), [Outcome::Partial]);
    assert_eq!(terminal(&done, &old_runtime), [Outcome::Partial]);
    let stale = id(&done, "slack.read.apply.skipped.stale");
    assert_eq!(terminal(&done, &stale), [Outcome::Blocked]);
    let p = shared.lock().unwrap();
    assert_eq!((p.cfg.clone(), p.data.clone(), p.checks.clone()), before);
    assert_eq!(host.message.as_ref().unwrap().text, notice);
    assert_eq!(rec.requests().len(), 2);
    assert_eq!(replacement.requests().len(), 2);
}

#[test]
fn add_detail_keeps_pending_success_and_failure_feedback_before_static_help() {
    let (mut p, _) = sample(vec![page(vec![], ""), page(vec![], "")]);
    let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
    p.on_msg(run(task));
    let failure = p.checks["add"].0.clone();
    for (state, expected) in [
        (None, None),
        (Some(Check::Running), Some("checking with Slack…")),
        (
            Some(Check::Passed("channel resolved".into())),
            Some("channel resolved"),
        ),
        (
            Some(failure),
            Some("No match for #PRIVATE_NAME in the returned lists"),
        ),
    ] {
        p.checks.remove("add");
        if let Some(state) = state {
            p.checks.insert("add".into(), (state, p.now));
        }
        // The section's detail as view text, its lines joined.
        let view = settings::channels_view(&p, 22, p.now);
        let section = radar_core::headless::text::section(&view);
        let detail = section.split("── detail").nth(1).and_then(|d| d.split_once('\n')).expect("the detail").1;
        let text = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(expected) = expected {
            let result_at = text.find(expected).unwrap_or_else(|| panic!("{text}"));
            assert!(
                result_at < text.find("Read another channel.").unwrap(),
                "{text}"
            );
            assert!(text.starts_with("LATEST CHANNEL CHECK"), "{text}");
            if matches!(&p.checks["add"].0, Check::Failed(_)) {
                assert!(text.contains("channel ID or link"), "{text}");
            }
        } else {
            assert!(text.starts_with("Read another channel."), "{text}");
            assert!(!text.contains("LATEST CHANNEL CHECK"), "{text}");
        }
    }
}
