//! Recorded-only channel lookup history and application ownership.

use std::sync::{Arc, Mutex};

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionDescriptor, ActionId, Origin, Outcome, Reason};
use radar_core::plugin::{Effect, Msg, Task};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

use super::{channel, channel_action};
use crate::client::{ChannelEnd, ChannelEvidence, ChannelKind, ChannelPhase, Recorded, Reply};
use crate::{Check, SlackMsg, SlackPlugin, Via, demo, settings};

const PREFIX: &str = "slack.settings.channel.resolve";
const ID: &str = "C0PRIVATE";

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn row(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "is_private": false, "is_member": true,
        "is_im": false, "is_archived": false, "topic": {"value": "PRIVATE_TOPIC"},
        "purpose": {"value": "PRIVATE_PURPOSE"}})
}

fn page(rows: Vec<Value>, cursor: &str) -> anyhow::Result<Reply> {
    ok(json!({"ok": true, "channels": rows, "response_metadata": {"next_cursor": cursor}}))
}

fn sample(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let mut p = SlackPlugin::new();
    let mut me = demo::sample().data.me.unwrap();
    me.team_id = "T0PRIVATE".into();
    me.url = "https://PRIVATE_WORKSPACE.slack.com/".into();
    me.scopes = vec!["PRIVATE_SCOPE".into()];
    assert!(p.install_me(me));
    p.cfg.token = "env:PRIVATE_REFERENCE".into();
    let rec = Recorded::new(replies);
    p.via = Via(Some(rec.clone()));
    (p, rec)
}

fn start(p: &mut SlackPlugin, key: &str, what: Result<&str, &str>) -> (ActionDescriptor, Task) {
    settings::check_channel(
        p,
        key.into(),
        what.map(str::to_string).map_err(str::to_string),
    )
    .into_iter()
    .find_map(|fx| match fx {
        Effect::SpawnAction { action, task } => {
            assert_eq!(action.kind.as_str(), PREFIX);
            assert!(action.subjects.is_empty());
            assert_eq!(action.metadata.count, None);
            Some((action, task))
        }
        _ => None,
    })
    .expect("independent channel task")
}

fn run(task: Task) -> Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

fn duplicate(msg: &Msg) -> Msg {
    let SlackMsg::Checked {
        stamp,
        key,
        requested_id,
        action,
        evidence,
        result,
    } = msg.downcast_ref::<SlackMsg>().unwrap()
    else {
        panic!("channel reply");
    };
    Box::new(SlackMsg::Checked {
        stamp: *stamp,
        key: key.clone(),
        requested_id: requested_id.clone(),
        action: action.clone(),
        evidence: *evidence,
        result: result.clone(),
    })
}

fn history(fx: &[Effect]) -> Vec<&Effect> {
    fx.iter()
        .filter(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
        .collect()
}

fn private_absent(text: &str) {
    for value in ["PRIVATE", "env:", "xoxp-", "slack.com", "C0PRIVATE"] {
        assert!(!text.contains(value), "history leaked {value}: {text}");
    }
}

fn report(fx: &[Effect], outcome: Outcome) {
    let reports: Vec<_> = fx
        .iter()
        .filter_map(|fx| {
            if let Effect::Report(got) = fx {
                Some(*got)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(reports, [outcome]);
    private_absent(&format!("{:?}", history(fx)));
    for fx in fx {
        if let Effect::Observe { action, .. } = fx {
            assert!(action.subjects.is_empty());
        }
    }
}

fn observation<'a>(
    fx: &'a [Effect],
    suffix: &str,
    outcome: Outcome,
    n: u32,
) -> &'a ActionDescriptor {
    let kind = format!("{PREFIX}.{suffix}");
    let got: Vec<_> = fx
        .iter()
        .filter_map(|fx| match fx {
            Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
                Some((action, outcome))
            }
            _ => None,
        })
        .collect();
    assert_eq!(got.len(), 1, "{fx:?}");
    assert_eq!(*got[0].1, outcome);
    assert_eq!(got[0].0.metadata.count, Some(n));
    got[0].0
}

fn skipped(fx: &[Effect]) {
    assert!(
        matches!(fx.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
        if action.kind.as_str() == "slack.read.apply.skipped.stale" && action.subjects.is_empty())
    );
    assert!(
        fx.iter()
            .all(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
    );
}

#[test]
fn id_result_is_unscoped_and_applies_without_history_in_cache() {
    let (mut p, rec) = sample(vec![ok(
        json!({"ok": true, "channel": row(ID, "PRIVATE_NAME")}),
    )]);
    let (_, task) = start(&mut p, "channel:PRIVATE_KEY", Ok(ID));
    assert!(rec.requests().is_empty());
    let fx = p.on_msg(run(task));
    report(&fx, Outcome::Succeeded);
    observation(&fx, "info.requests", Outcome::Succeeded, 1);
    observation(&fx, "info.returned", Outcome::Succeeded, 1);
    observation(&fx, "info.membership.explicit", Outcome::Succeeded, 1);
    assert_eq!(
        p.data.channels[ID].info.as_ref().unwrap().name,
        "PRIVATE_NAME"
    );
    assert!(matches!(
        p.checks.get("channel:PRIVATE_KEY"),
        Some((Check::Passed(_), _))
    ));
    assert!(!p.snapshot().unwrap().to_string().contains(PREFIX));
    let req = &rec.requests()[0];
    assert_eq!(rec.requests().len(), 1);
    assert_eq!(req.method, "conversations.info");
    assert_eq!(req.params, [("channel".into(), ID.into())]);
    assert!(!req.write && req.body.is_none());
}

#[test]
fn users_match_membership_is_list_derived_and_counts_whole_matched_page() {
    let mut matched = row(ID, "PRIVATE_NAME");
    matched.as_object_mut().unwrap().remove("is_member");
    let (mut p, rec) = sample(vec![page(
        vec![matched, row(ID, "PRIVATE_NAME"), row("C1", "other")],
        "PRIVATE_UNREAD_CURSOR",
    )]);
    let (_, task) = start(&mut p, "add", Err("#PRIVATE_NAME"));
    let fx = p.on_msg(run(task));
    report(&fx, Outcome::Succeeded);
    observation(&fx, "yours.returned", Outcome::Succeeded, 3);
    observation(&fx, "yours.membership.list_derived", Outcome::Succeeded, 1);
    assert_eq!(p.cfg.channels.len(), 1);
    assert!(p.data.channels[ID].info.as_ref().unwrap().is_member);
    assert_eq!(rec.requests().len(), 1);
    assert_eq!(rec.requests()[0].method, "users.conversations");
    assert!(!p.snapshot().unwrap().to_string().contains(PREFIX));
}

#[test]
fn complete_negative_and_incomplete_negative_keep_original_history() {
    for incomplete in [false, true] {
        let first = if incomplete {
            ok(json!({"ok": true}))
        } else {
            page(vec![], "")
        };
        let (mut p, rec) = sample(vec![first, page(vec![], "")]);
        let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
        let fx = p.on_msg(run(task));
        let outcome = if incomplete {
            Outcome::Partial
        } else {
            Outcome::Failed
        };
        report(&fx, outcome);
        observation(
            &fx,
            if incomplete {
                "no_match.incomplete"
            } else {
                "no_match"
            },
            outcome,
            1,
        );
        observation(&fx, "public.requests", Outcome::Succeeded, 1);
        observation(&fx, "public.no_match", Outcome::Succeeded, 1);
        assert!(!format!("{:?}", history(&fx)).contains("request.failed"));
        let Check::Failed(text) = &p.checks["add"].0 else {
            panic!("negative lookup feedback");
        };
        assert!(text.contains("PRIVATE_NAME"));
        assert!(p.cfg.channels.is_empty());
        assert_eq!(rec.requests().len(), 2);
    }
}

#[test]
fn later_request_error_retains_only_completed_page_counts_and_raw_error_stays_ui_only() {
    let (mut p, rec) = sample(vec![
        page(vec![row("C1", "other")], ""),
        page(
            vec![row("C2", "other"), row("C2", "other")],
            "PRIVATE_CURSOR",
        ),
        Err(anyhow::anyhow!("PRIVATE_ERROR xoxp-PRIVATE_TOKEN")),
    ]);
    let (_, task) = start(&mut p, "add", Err("PRIVATE_NAME"));
    let fx = p.on_msg(run(task));
    report(&fx, Outcome::Failed);
    observation(&fx, "yours.requests", Outcome::Succeeded, 1);
    observation(&fx, "yours.returned", Outcome::Succeeded, 1);
    observation(&fx, "public.requests", Outcome::Succeeded, 1);
    observation(&fx, "public.returned", Outcome::Succeeded, 2);
    observation(&fx, "public.request.failed", Outcome::Failed, 1);
    assert!(!format!("{:?}", history(&fx)).contains("resolve.no_match"));
    assert!(matches!(&p.checks["add"].0, Check::Failed(error) if error.contains("PRIVATE_ERROR")));
    assert_eq!(rec.requests().len(), 3);
}

#[test]
fn malformed_info_is_not_applied_but_keeps_partial_lookup_history() {
    for body in [
        json!({"ok": true}),
        json!({"ok": true, "channel": row("C0OTHER", "PRIVATE_NAME")}),
    ] {
        let (mut p, _) = sample(vec![ok(body)]);
        let (_, task) = start(&mut p, "add", Ok(ID));
        let fx = p.on_msg(run(task));
        report(&fx, Outcome::Partial);
        observation(&fx, "info.identity.incomplete", Outcome::Partial, 1);
        observation(&fx, "apply.blocked.identity", Outcome::Blocked, 1);
        assert!(p.cfg.channels.is_empty());
        assert!(p.data.channels.is_empty());
    }
}

#[test]
fn local_cap_is_partial_and_empty_evidence_cannot_invent_progress() {
    let evidence = ChannelEvidence {
        phases: [
            Some(ChannelPhase {
                kind: ChannelKind::Yours,
                requests: 100,
                returned: 7,
                defaulted_fields: 0,
                identity_gaps: 0,
                incomplete_pages: 0,
                end: ChannelEnd::LocalCap,
                membership: None,
            }),
            None,
        ],
    };
    let fx = channel(&channel_action(), &evidence);
    report(&fx, Outcome::Partial);
    assert!(
        observation(&fx, "yours.cap", Outcome::Partial, 1)
            .metadata
            .truncated
    );
    observation(&fx, "yours.requests", Outcome::Succeeded, 100);
    observation(&fx, "yours.returned", Outcome::Succeeded, 7);
    assert!(!format!("{fx:?}").contains("request.failed"));
    assert!(!format!("{fx:?}").contains("no_match"));
    let unknown = channel(&channel_action(), &ChannelEvidence::default());
    assert!(matches!(
        unknown.as_slice(),
        [Effect::Report(Outcome::Unknown)]
    ));
}

#[test]
fn duplicate_and_reversed_same_key_completions_keep_original_outcomes() {
    let (mut p, _) = sample(vec![ok(
        json!({"ok": true, "channel": row(ID, "PRIVATE_OLD")}),
    )]);
    let (_, older) = start(&mut p, "channel:PRIVATE_KEY", Ok(ID));
    p.via = Via(Some(Recorded::new(vec![ok(
        json!({"ok": true, "channel": row(ID, "PRIVATE_NEW")}),
    )])));
    let (_, newer) = start(&mut p, "channel:PRIVATE_KEY", Ok(ID));
    let newer = run(newer);
    let again = duplicate(&newer);
    report(&p.on_msg(newer), Outcome::Succeeded);
    let before = (p.cfg.clone(), p.data.clone(), p.checks.clone());
    for msg in [run(older), again] {
        let fx = p.on_msg(msg);
        report(&fx, Outcome::Succeeded);
        skipped(&fx);
        assert_eq!((p.cfg.clone(), p.data.clone(), p.checks.clone()), before);
    }
    assert_eq!(
        p.data.channels[ID].info.as_ref().unwrap().name,
        "PRIVATE_NEW"
    );
}

#[test]
fn independent_keys_and_adds_apply_in_both_orders_without_cross_cancellation() {
    for add in [false, true] {
        for reverse in [false, true] {
            let (mut p, _) = sample(vec![ok(
                json!({"ok": true, "channel": row("C00000001", "one")}),
            )]);
            let (_, one) = start(
                &mut p,
                if add { "add" } else { "channel:C00000001" },
                Ok("C00000001"),
            );
            p.via = Via(Some(Recorded::new(vec![ok(
                json!({"ok": true, "channel": row("C00000002", "two")}),
            )])));
            let (_, two) = start(
                &mut p,
                if add { "add" } else { "channel:C00000002" },
                Ok("C00000002"),
            );
            let tasks = if reverse { [two, one] } else { [one, two] };
            for task in tasks {
                report(&p.on_msg(run(task)), Outcome::Succeeded);
            }
            assert!(
                p.data.channels.contains_key("C00000001")
                    && p.data.channels.contains_key("C00000002")
            );
            assert_eq!(p.cfg.channels.len(), if add { 2 } else { 0 });
        }
    }
}

#[test]
fn local_refusal_has_no_task_or_report_and_stale_reply_keeps_its_result() {
    let (mut p, rec) = sample(vec![ok(
        json!({"ok": true, "channel": row(ID, "PRIVATE_NAME")}),
    )]);
    let (_, task) = start(&mut p, "channel:PRIVATE_KEY", Ok(ID));
    p.via = Via(None);
    p.cfg.token.clear();
    let refusal = settings::check_channel(&mut p, "channel:PRIVATE_KEY".into(), Ok(ID.into()));
    assert!(!refusal.iter().any(|fx| matches!(
        fx,
        Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::Report(_)
    )));
    assert_eq!(
        observation(&refusal, "unavailable", Outcome::Blocked, 1)
            .metadata
            .reason,
        Some(Reason::Unavailable)
    );
    assert!(rec.requests().is_empty());
    let before = (p.cfg.clone(), p.data.clone(), p.checks.clone());
    let fx = p.on_msg(run(task));
    report(&fx, Outcome::Succeeded);
    skipped(&fx);
    assert_eq!((p.cfg, p.data, p.checks), before);
}

enum Command {
    Start,
    Refuse,
    All,
    Name,
}
struct ChannelHost(Arc<Mutex<SlackPlugin>>);
impl Plugin for ChannelHost {
    fn id(&self) -> &'static str {
        "slack"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        self.0.lock().unwrap().config()
    }
    fn snapshot(&self) -> Option<Value> {
        self.0.lock().unwrap().snapshot()
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let mut p = self.0.lock().unwrap();
        match msg.downcast::<Command>() {
            Err(msg) => p.on_msg(msg),
            Ok(command) => {
                if matches!(*command, Command::Refuse) {
                    p.via = Via(None);
                    p.cfg.token.clear();
                }
                let mut fx = vec![Effect::Journal(ActionDescriptor::new(
                    "test.enclosing",
                    Origin::Host,
                ))];
                if matches!(*command, Command::All) {
                    fx.extend(settings::check_token(&mut p));
                    fx.extend(settings::on_bulk(&mut p, "ch-check-all"));
                } else if matches!(*command, Command::Name) {
                    fx.extend(settings::check_channel(
                        &mut p,
                        "add".into(),
                        Err("PRIVATE_NAME".into()),
                    ));
                } else {
                    fx.extend(settings::check_channel(
                        &mut p,
                        "channel:PRIVATE_KEY".into(),
                        Ok(ID.into()),
                    ));
                }
                fx.push(Effect::Report(Outcome::Succeeded));
                fx
            }
        }
    }
}

fn records(fx: &[HostEffect]) -> Vec<JournalRecord> {
    fx.iter()
        .filter_map(|fx| {
            if let HostEffect::Journal(record) = fx {
                Some(record.clone())
            } else {
                None
            }
        })
        .collect()
}
fn terminal(records: &[JournalRecord], target: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome { id, outcome, .. } if id == target && outcome.is_terminal() => {
                Some(*outcome)
            }
            _ => None,
        })
        .collect()
}
fn id(records: &[JournalRecord], kind: &str) -> ActionId {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == kind => {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn host_stale_success_and_failure_keep_original_unscoped_semantic_outcome() {
    for failed in [false, true] {
        let reply = if failed {
            Err(anyhow::anyhow!("PRIVATE_ERROR"))
        } else {
            ok(json!({"ok": true, "channel": row(ID, "PRIVATE_NAME")}))
        };
        let (p, rec) = sample(vec![reply]);
        let shared = Arc::new(Mutex::new(p));
        let mut host = Host::new(vec![Box::new(ChannelHost(shared.clone()))], demo::NOW).unwrap();
        host.enable_journal();
        let fx = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Command::Start),
        });
        let begun = records(&fx);
        let parent = id(&begun, "test.enclosing");
        let lookup = id(&begun, PREFIX);
        assert!(terminal(&begun, &lookup).is_empty());
        assert_eq!(terminal(&begun, &parent), [Outcome::Succeeded]);
        assert!(rec.requests().is_empty());
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
        shared
            .lock()
            .unwrap()
            .replace_token("env:PRIVATE_REPLACEMENT".into());
        let before = shared.lock().unwrap().data.clone();
        let fx = host.update(Event::Tracked {
            action: runtime.clone(),
            semantic,
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: run(task),
            }),
        });
        let done = records(&fx);
        let expected = if failed {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        };
        assert_eq!(terminal(&done, &lookup), [expected]);
        assert_eq!(terminal(&done, &runtime), [expected]);
        assert!(terminal(&done, &parent).is_empty());
        let stale = id(&done, "slack.read.apply.skipped.stale");
        assert_eq!(terminal(&done, &stale), [Outcome::Blocked]);
        for record in &done {
            if let JournalRecord::Begin { descriptor, .. } = record {
                assert!(descriptor.subjects.is_empty());
            }
        }
        assert_eq!(shared.lock().unwrap().data, before);
        private_absent(&format!("{begun:?}{done:?}"));
        assert_eq!(rec.requests().len(), 1);
    }
}

#[test]
fn host_local_refusal_is_independent_and_unstarted() {
    let (p, rec) = sample(vec![]);
    let mut host = Host::new(
        vec![Box::new(ChannelHost(Arc::new(Mutex::new(p))))],
        demo::NOW,
    )
    .unwrap();
    host.enable_journal();
    let fx = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Command::Refuse),
    });
    let records = records(&fx);
    let parent = id(&records, "test.enclosing");
    let refusal = id(&records, &format!("{PREFIX}.unavailable"));
    assert_eq!(terminal(&records, &parent), [Outcome::Succeeded]);
    assert_eq!(terminal(&records, &refusal), [Outcome::Blocked]);
    assert!(!records.iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, outcome: Outcome::Started, .. } if id == &refusal)));
    assert!(rec.requests().is_empty());
    private_absent(&format!("{records:?}"));
}

mod acceptance;
mod feedback;
