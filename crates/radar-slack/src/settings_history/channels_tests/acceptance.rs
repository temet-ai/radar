//! Identity-only settings acceptance; all tasks use recordings, and follow-up
//! refresh tasks are inspected without polling. No live reproduction is claimed.

use super::*;
use crate::client::ChannelInfo;

const OTHER: &str = "G0PRIVATE";
const BAD_ID: &str = "C1234567890123";
const REJECTION: &str =
    "Slack returned an unusable channel identity; no channel settings were changed";

fn keep_channel(p: &mut SlackPlugin, id: &str) {
    let mut configured = demo::sample().cfg.channels.remove(0);
    configured.id = id.into();
    configured.name = format!("PRIVATE_OLD_{id}");
    configured.alias = format!("kept{}", p.cfg.channels.len());
    configured.watched = false;
    p.cfg.channels.push(configured);
    p.data.channels.entry(id.into()).or_default().info = Some(ChannelInfo {
        id: id.into(),
        name: format!("PRIVATE_CACHED_{id}"),
        topic: "PRIVATE_CACHED_TOPIC".into(),
        purpose: "PRIVATE_CACHED_PURPOSE".into(),
        ..Default::default()
    });
}

fn persisted_bytes(p: &SlackPlugin) -> (Vec<u8>, Vec<u8>) {
    (
        toml::to_string(&p.config().unwrap()).unwrap().into_bytes(),
        serde_json::to_vec(&p.snapshot().unwrap()).unwrap(),
    )
}

fn rejected(p: &SlackPlugin, key: &str, fx: &[Effect]) {
    let action = observation(fx, "apply.blocked.identity", Outcome::Blocked, 1);
    assert!(action.subjects.is_empty());
    assert_eq!(action.metadata.reason, None);
    assert!(!action.metadata.redacted && !action.metadata.truncated);
    assert_eq!(action.metadata.omitted_subjects, 0);
    assert!(matches!(&p.checks[key].0, Check::Failed(text) if text == REJECTION));
    assert_eq!(
        fx.iter()
            .filter_map(|fx| match fx {
                Effect::Notice(message) if message.tone == radar_core::view::chrome::NoticeTone::Error => Some(&message.text),
                _ => None,
            })
            .count(),
        1
    );
    assert!(fx.iter().all(|fx| matches!(
        fx,
        Effect::Report(_) | Effect::Observe { .. } | Effect::Notice(_)
    )));
    assert!(!format!("{:?}", history(fx)).contains("request.failed"));
    private_absent(&format!("{:?}", history(fx)));
}

fn bad_rows() -> Vec<Value> {
    let mut rows = vec![
        Value::Null,
        json!(false),
        json!(12),
        json!([]),
        json!("PRIVATE_ROW"),
    ];
    let mut absent = row(ID, "PRIVATE_NAME");
    absent.as_object_mut().unwrap().remove("id");
    rows.push(absent);
    for id in [
        Value::Null,
        json!(12),
        json!(false),
        json!([]),
        json!({}),
        json!(""),
        json!("   "),
        json!("C0PRIVATE "),
        json!(" C0PRIVATE"),
        json!("C0PRIVAT\n"),
        json!("C0private"),
        json!("X0PRIVATE"),
        json!("C1"),
        json!(BAD_ID),
        json!("C0PRI/VAT"),
        json!("https://PRIVATE_WORKSPACE.slack.com/archives/C0PRIVATE"),
        json!(OTHER),
    ] {
        let mut value = row(ID, "PRIVATE_NAME");
        value["id"] = id;
        rows.push(value);
    }
    rows
}

#[test]
fn malformed_or_mismatched_info_cannot_add_or_modify_either_cached_identity() {
    let mut bodies = vec![json!({"ok": true})];
    bodies.extend(
        bad_rows()
            .into_iter()
            .map(|channel| json!({"ok": true, "channel": channel})),
    );
    for body in bodies {
        for key in ["add", "channel:C0PRIVATE"] {
            let parsed_id = body
                .pointer("/channel/id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let (mut p, rec) = sample(vec![ok(body.clone())]);
            keep_channel(&mut p, ID);
            keep_channel(&mut p, OTHER);
            p.checks.insert(
                format!("channel:{OTHER}"),
                (Check::Passed("kept".into()), p.now),
            );
            let (_, task) = start(&mut p, key, Ok(ID));
            let before = persisted_bytes(&p);
            let held_rows = p.section_rows;
            assert!(rec.requests().is_empty());
            let message = run(task);
            // The provider/parser result and evidence reach settings unchanged.
            let SlackMsg::Checked {
                requested_id,
                result: Ok(crate::Checked::Channel(info)),
                ..
            } = message.downcast_ref::<SlackMsg>().unwrap()
            else {
                panic!("parsed info");
            };
            assert_eq!(requested_id.as_deref(), Some(ID));
            assert_eq!(info.id, parsed_id);
            let fx = p.on_msg(message);
            report(&fx, Outcome::Partial);
            observation(&fx, "info.requests", Outcome::Succeeded, 1);
            assert_eq!(
                p.cfg
                    .channels
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                [ID, OTHER],
                "rejected response must not create or replace a registry identity"
            );
            rejected(&p, key, &fx);
            assert_eq!(persisted_bytes(&p), before, "{body}");
            assert_eq!(p.section_rows, held_rows);
            assert!(
                matches!(&p.checks[&format!("channel:{OTHER}")].0, Check::Passed(text) if text == "kept")
            );
            assert_eq!(rec.requests().len(), 1);
            assert_eq!(rec.requests()[0].params, [("channel".into(), ID.into())]);
        }
    }
}

#[test]
fn matched_name_needs_usable_id_without_changing_successful_lookup_evidence() {
    for id in [
        Value::Null,
        json!(5),
        json!(""),
        json!(" "),
        json!("C1"),
        json!(BAD_ID),
        json!("X0PRIVATE"),
    ] {
        for public in [false, true] {
            let mut matched = row(ID, "PRIVATE_NAME");
            matched["id"] = id.clone();
            let mut replies = vec![];
            if public {
                replies.push(page(vec![], ""));
            }
            // The actual first match is unusable; another row with that name is
            // not authority to repair or substitute its identity in settings.
            replies.push(page(vec![matched, row(ID, "PRIVATE_NAME")], ""));
            let (mut p, rec) = sample(replies);
            keep_channel(&mut p, OTHER);
            let (_, task) = start(&mut p, "add", Err("#PRIVATE_NAME"));
            let before = persisted_bytes(&p);
            let message = run(task);
            let SlackMsg::Checked { requested_id, .. } =
                message.downcast_ref::<SlackMsg>().unwrap()
            else {
                panic!("name result");
            };
            assert_eq!(requested_id, &None);
            let fx = p.on_msg(message);
            let expected = if id.as_str().is_none_or(|id| id.trim().is_empty()) {
                Outcome::Partial
            } else {
                Outcome::Succeeded
            };
            report(&fx, expected);
            observation(
                &fx,
                if public {
                    "public.found"
                } else {
                    "yours.found"
                },
                expected,
                1,
            );
            rejected(&p, "add", &fx);
            assert_eq!(persisted_bytes(&p), before);
            assert_eq!(rec.requests().len(), if public { 2 } else { 1 });
        }
    }
}

#[test]
fn supported_direct_and_name_matches_keep_legacy_add_check_and_optional_defaults() {
    for id in ["C00000001", "G000000001", "D00000000001"] {
        assert!(crate::config::is_channel_id(id));
        for add in [false, true] {
            for name in [false, true] {
                for full in [false, true] {
                    let channel = if full {
                        row(id, "PRIVATE_NAME")
                    } else if name {
                        json!({"id": id, "name": "PRIVATE_NAME"})
                    } else {
                        json!({"id": id})
                    };
                    let reply = if name {
                        page(vec![channel], "")
                    } else {
                        ok(json!({"ok": true, "channel": channel}))
                    };
                    let (mut p, rec) = sample(vec![reply]);
                    if !add {
                        keep_channel(&mut p, id);
                    }
                    let key = if add {
                        "add".into()
                    } else {
                        format!("channel:{id}")
                    };
                    let (_, task) = start(
                        &mut p,
                        &key,
                        if name { Err("PRIVATE_NAME") } else { Ok(id) },
                    );
                    let fx = p.on_msg(run(task));
                    report(
                        &fx,
                        if full {
                            Outcome::Succeeded
                        } else {
                            Outcome::Partial
                        },
                    );
                    assert!(!format!("{:?}", history(&fx)).contains("apply.blocked"));
                    assert_eq!(p.cfg.channels.len(), 1);
                    assert_eq!(p.cfg.channels[0].id, id);
                    let info = p.data.channels[id].info.as_ref().unwrap();
                    assert_eq!(info.id, id);
                    assert_eq!(info.name, if name || full { "PRIVATE_NAME" } else { "" });
                    assert!(matches!(
                        p.checks[&format!("channel:{id}")].0,
                        Check::Passed(_)
                    ));
                    assert_eq!(
                        fx.iter()
                            .filter(|fx| matches!(
                                fx,
                                Effect::Spawn(_) | Effect::SpawnAction { .. }
                            ))
                            .count()
                            > 0,
                        add
                    );
                    // Follow-up refresh effects are deliberately never polled.
                    assert_eq!(rec.requests().len(), 1);
                    assert!(!p.snapshot().unwrap().to_string().contains("requested_id"));
                }
            }
        }
    }
}

#[test]
fn direct_target_is_captured_even_if_the_local_status_key_or_config_now_matches_reply() {
    for accepted in [false, true] {
        let returned = if accepted { ID } else { OTHER };
        let (mut p, rec) = sample(vec![ok(
            json!({"ok": true, "channel": row(returned, "PRIVATE_NEW")}),
        )]);
        keep_channel(&mut p, ID);
        keep_channel(&mut p, OTHER);
        // A display key is not the request target. Also replace the requested
        // config entry before completion without changing the token generation.
        let key = format!("channel:{OTHER}");
        let (_, task) = start(&mut p, &key, Ok(ID));
        p.cfg.channels[0].id = "D0PRIVATE".into();
        let before = persisted_bytes(&p);
        let fx = p.on_msg(run(task));
        report(
            &fx,
            if accepted {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
        );
        if accepted {
            assert_eq!(
                p.data.channels[ID].info.as_ref().unwrap().name,
                "PRIVATE_NEW"
            );
        } else {
            rejected(&p, &key, &fx);
            assert_eq!(persisted_bytes(&p), before);
        }
        assert_eq!(rec.requests()[0].params, [("channel".into(), ID.into())]);
    }
}

#[test]
fn rejected_add_is_single_use_and_does_not_cancel_a_valid_independent_add() {
    for name in [false, true] {
        for reverse in [false, true] {
            let bad = if name {
                page(vec![row(BAD_ID, "PRIVATE_NAME")], "")
            } else {
                ok(json!({"ok": true}))
            };
            let (mut p, bad_rec) = sample(vec![bad]);
            let (_, bad_task) = start(
                &mut p,
                "add",
                if name { Err("PRIVATE_NAME") } else { Ok(ID) },
            );
            let good_rec = Recorded::new(vec![ok(
                json!({"ok": true, "channel": row(OTHER, "PRIVATE_GOOD")}),
            )]);
            p.via = Via(Some(good_rec.clone()));
            let (_, good_task) = start(&mut p, "add", Ok(OTHER));
            let bad = run(bad_task);
            let bad_again = duplicate(&bad);
            let good = run(good_task);
            let good_again = duplicate(&good);
            let messages = if reverse {
                [(true, good), (false, bad)]
            } else {
                [(false, bad), (true, good)]
            };
            for (valid, message) in messages {
                let before = persisted_bytes(&p);
                let fx = p.on_msg(message);
                report(
                    &fx,
                    if valid || name {
                        Outcome::Succeeded
                    } else {
                        Outcome::Partial
                    },
                );
                if !valid {
                    rejected(&p, "add", &fx);
                    assert_eq!(persisted_bytes(&p), before);
                }
            }
            assert_eq!(p.cfg.channels.len(), 1);
            assert_eq!(p.cfg.channels[0].id, OTHER);
            let before = (persisted_bytes(&p), p.checks.clone());
            for message in [bad_again, good_again] {
                let fx = p.on_msg(message);
                skipped(&fx);
                assert!(!format!("{:?}", history(&fx)).contains("apply.blocked.identity"));
                assert_eq!((persisted_bytes(&p), p.checks.clone()), before);
            }
            assert_eq!(bad_rec.requests().len(), 1);
            assert_eq!(good_rec.requests().len(), 1);
        }
    }
}

#[test]
fn stale_malformed_result_is_skipped_without_failing_replacement_state() {
    for add in [false, true] {
        for token in [false, true] {
            if add && !token {
                continue;
            } // Adds do not supersede one another.
            let (mut p, _) = sample(vec![ok(json!({"ok": true}))]);
            let key = if add { "add" } else { "channel:C0PRIVATE" };
            let (_, old) = start(&mut p, key, Ok(ID));
            if token {
                let me = p.data.me.clone().unwrap();
                p.replace_token("env:PRIVATE_REPLACEMENT".into());
                assert!(p.install_me(me));
            }
            p.via = Via(Some(Recorded::new(vec![ok(
                json!({"ok": true, "channel": row(ID, "PRIVATE_NEW")}),
            )])));
            let (_, fresh) = start(&mut p, key, Ok(ID));
            let before = (persisted_bytes(&p), p.checks.clone());
            let fx = p.on_msg(run(old));
            report(&fx, Outcome::Partial);
            skipped(&fx);
            assert!(!format!("{:?}", history(&fx)).contains("apply.blocked.identity"));
            assert_eq!((persisted_bytes(&p), p.checks.clone()), before);
            let fx = p.on_msg(run(fresh));
            report(&fx, Outcome::Succeeded);
            assert_eq!(
                p.data.channels[ID].info.as_ref().unwrap().name,
                "PRIVATE_NEW"
            );
        }
    }
}

fn no_host_persistence_or_refresh(effects: &[HostEffect]) {
    for effect in effects {
        match effect {
            HostEffect::Tracked { effect, .. } => {
                no_host_persistence_or_refresh(std::slice::from_ref(effect))
            }
            HostEffect::Journal(_) => {}
            other => panic!("rejected settings result produced a host side effect: {other:?}"),
        }
    }
}

#[test]
fn host_independent_reads_keep_outcomes_and_siblings_separate_from_identity_block() {
    for partial in [false, true] {
        // A syntactically unsupported, but faithfully parsed, ID still gives
        // Succeeded read evidence. radar's current grammar is not Slack's schema.
        let target = if partial { ID } else { BAD_ID };
        let bad = if partial {
            json!({"ok": true})
        } else {
            json!({"ok": true, "channel": row(target, "PRIVATE_BAD")})
        };
        let (mut p, rec) = sample(vec![
            Err(anyhow::anyhow!("PRIVATE_AUTH_ERROR")),
            ok(bad),
            ok(json!({"ok": true, "channel": row(OTHER, "PRIVATE_GOOD")})),
        ]);
        keep_channel(&mut p, target);
        keep_channel(&mut p, OTHER);
        let shared = Arc::new(Mutex::new(p));
        let mut host = Host::new(vec![Box::new(ChannelHost(shared.clone()))], demo::NOW).unwrap();
        host.enable_journal();
        let fx = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Command::All),
        });
        let begun = records(&fx);
        let parent = id(&begun, "test.enclosing");
        let auth = id(&begun, "slack.settings.check.workspace");
        let lookup_ids: Vec<_> = begun
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == PREFIX =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(lookup_ids.len(), 2);
        assert_eq!(terminal(&begun, &parent), [Outcome::Succeeded]);
        assert!(rec.requests().is_empty());
        let tasks: Vec<_> = fx
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::Tracked {
                    action,
                    semantic: Some(semantic),
                    effect,
                } => match *effect {
                    HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let messages: Vec<_> = tasks
            .into_iter()
            .map(|(runtime, semantic, task)| (runtime, semantic, run(task)))
            .collect();
        assert_eq!(messages.len(), 3);
        let mut messages = messages.into_iter();
        let auth_message = messages.next().unwrap();
        let (runtime, semantic, message) = messages.next().unwrap();
        assert_eq!(semantic, lookup_ids[0]);
        let before = persisted_bytes(&shared.lock().unwrap());
        let fx = host.update(Event::Tracked {
            action: runtime.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        no_host_persistence_or_refresh(&fx);
        assert_eq!(persisted_bytes(&shared.lock().unwrap()), before);
        let rejected = records(&fx);
        let expected = if partial {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        assert_eq!(terminal(&rejected, &semantic), [expected]);
        assert_eq!(terminal(&rejected, &runtime), [expected]);
        let blocked = id(&rejected, &format!("{PREFIX}.apply.blocked.identity"));
        assert_eq!(terminal(&rejected, &blocked), [Outcome::Blocked]);
        assert!(terminal(&rejected, &auth).is_empty());
        assert!(terminal(&rejected, &parent).is_empty());
        assert!(terminal(&rejected, &lookup_ids[1]).is_empty());
        assert!(!format!("{rejected:?}").contains("request.failed"));
        for record in &rejected {
            if let JournalRecord::Begin { descriptor, .. } = record {
                assert!(descriptor.subjects.is_empty());
            }
        }
        let (sibling_runtime, sibling, message) = messages.next().unwrap();
        assert_eq!(sibling, lookup_ids[1]);
        let fx = host.update(Event::Tracked {
            action: sibling_runtime.clone(),
            semantic: Some(sibling.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        let sibling_records = records(&fx);
        assert_eq!(terminal(&sibling_records, &sibling), [Outcome::Succeeded]);
        assert_eq!(
            terminal(&sibling_records, &sibling_runtime),
            [Outcome::Succeeded]
        );
        assert!(terminal(&sibling_records, &semantic).is_empty());
        assert!(terminal(&sibling_records, &auth).is_empty());
        assert_eq!(
            shared.lock().unwrap().data.channels[OTHER]
                .info
                .as_ref()
                .unwrap()
                .name,
            "PRIVATE_GOOD"
        );
        let (auth_runtime, auth_semantic, message) = auth_message;
        assert_eq!(auth_semantic, auth);
        let fx = host.update(Event::Tracked {
            action: auth_runtime,
            semantic: Some(auth.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        let auth_records = records(&fx);
        assert_eq!(terminal(&auth_records, &auth), [Outcome::Failed]);
        assert!(terminal(&auth_records, &semantic).is_empty());
        assert!(terminal(&auth_records, &sibling).is_empty());
        private_absent(&format!(
            "{begun:?}{rejected:?}{sibling_records:?}{auth_records:?}"
        ));
        assert_eq!(rec.requests().len(), 3);
    }
}
