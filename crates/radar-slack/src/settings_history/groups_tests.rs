//! Recorded group loads and local cache use; no credentials or live services.

use std::sync::{Arc, Mutex};

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionDescriptor, ActionId, Origin, Outcome, Reason};
use radar_core::plugin::{Effect, Msg, Task};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

use super::{groups, groups_action, groups_cached};
use crate::client::{GroupsRead, Recorded, Reply, UserGroup};
use crate::{Check, SlackMsg, SlackPlugin, Via, demo, settings};

const PREFIX: &str = "slack.settings.groups.load";

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn body() -> Value {
    json!({"ok": true, "usergroups": [
        {"id": "S0PRIVATE", "name": "PRIVATE_NAME", "handle": "PRIVATE_HANDLE", "users": ["U0PRIVATE"]},
        {"id": "S0PRIVATE", "name": "PRIVATE_NAME", "handle": "PRIVATE_HANDLE", "users": ["U0PRIVATE"]}
    ]})
}

fn sample(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let mut p = SlackPlugin::new();
    let mut me = demo::sample().data.me.unwrap();
    me.team_id = "T0PRIVATE".into();
    assert!(p.install_me(me));
    p.data.groups = vec![UserGroup {
        id: "S0PRIVATE_CACHED".into(),
        ..Default::default()
    }];
    p.sync.groups_at = 123;
    let recorded = Recorded::new(replies);
    p.via = Via(Some(recorded.clone()));
    (p, recorded)
}

fn run(task: Task) -> Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

fn start(p: &mut SlackPlugin, pick: bool) -> (ActionDescriptor, Task) {
    let effects = settings::fetch_groups(p, pick);
    assert_eq!(effects.len(), 2);
    assert!(matches!(effects[0], Effect::Notify(_)));
    let Effect::SpawnAction { action, task } = effects.into_iter().nth(1).unwrap() else {
        panic!("one isolated group task");
    };
    assert_eq!(action.kind.as_str(), PREFIX);
    assert!(action.subjects.is_empty());
    assert_eq!(action.metadata.count, None);
    assert!(!action.metadata.redacted);
    (action, task)
}

fn private_absent(text: &str) {
    for marker in ["PRIVATE", "env:", "xoxp-"] {
        assert!(!text.contains(marker), "history leaked content: {text}");
    }
}

fn history(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
        .collect()
}

fn report(effects: &[Effect], outcome: Outcome) {
    let reports: Vec<_> = effects
        .iter()
        .filter_map(|fx| match fx {
            Effect::Report(got) => Some(*got),
            _ => None,
        })
        .collect();
    assert_eq!(reports, [outcome]);
    private_absent(&format!("{:?}", history(effects)));
}

fn observation<'a>(
    effects: &'a [Effect],
    suffix: &str,
    outcome: Outcome,
    n: u32,
) -> &'a ActionDescriptor {
    let kind = format!("{PREFIX}.{suffix}");
    let found: Vec<_> = effects
        .iter()
        .filter_map(|fx| match fx {
            Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
                Some((action, outcome))
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "{effects:?}");
    assert_eq!(*found[0].1, outcome);
    let action = found[0].0;
    assert_eq!(action.metadata.count, Some(n));
    assert!(
        action.subjects.is_empty(),
        "cached workspace is not credential ownership"
    );
    assert!(!action.metadata.redacted);
    private_absent(&format!("{action:?}"));
    action
}

#[test]
fn requested_group_results_count_occurrences_and_preserve_both_application_modes() {
    for pick in [false, true] {
        let (mut p, recorded) = sample(vec![ok(body())]);
        p.cfg.token = "env:PRIVATE_REFERENCE".into();
        let (_, task) = start(&mut p, pick);
        assert!(recorded.requests().is_empty());
        let effects = p.on_msg(run(task));
        report(&effects, Outcome::Succeeded);
        observation(&effects, "request", Outcome::Succeeded, 1);
        observation(&effects, "returned", Outcome::Succeeded, 2);
        assert_eq!(history(&effects).len(), 3);
        assert_eq!(p.data.groups.len(), 2);
        assert_eq!(p.data.groups[0], p.data.groups[1]);
        assert_eq!(p.sync.groups_at, 123);
        assert!(matches!(
            p.checks.get("groups"),
            Some((Check::Passed(_), _))
        ));
        assert_eq!(
            effects
                .iter()
                .filter(|fx| matches!(fx, Effect::Menu(_)))
                .count(),
            usize::from(pick)
        );
        assert!(!p.snapshot().unwrap().to_string().contains(PREFIX));
        let requests = recorded.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "usergroups.list");
        assert_eq!(
            requests[0].params,
            [("include_users".into(), "true".into())]
        );
        assert!(!requests[0].write && requests[0].body.is_none());
    }
}

#[test]
fn observed_gaps_make_history_partial_without_changing_permissive_application() {
    for (body, returned, collection, entries) in [
        (json!({"ok": true, "usergroups": []}), 0, false, 0),
        (json!({"ok": true}), 0, true, 0),
        (json!({"ok": true, "usergroups": null}), 0, true, 0),
        (
            json!({"ok": true, "usergroups": [null, {"id": "", "users": [null, ""]}, {"id": "good", "users": []}]}),
            3,
            false,
            2,
        ),
        // Existing evidence does not validate handle/name/id shape or paging.
        (
            json!({"ok": true, "usergroups": [{"id": " ", "users": [" "], "handle": false}], "response_metadata": {"next_cursor": "PRIVATE_CURSOR"}}),
            1,
            false,
            0,
        ),
    ] {
        let (mut p, recorded) = sample(vec![ok(body)]);
        let (_, task) = start(&mut p, true);
        let outcome = if collection || entries != 0 {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        let effects = p.on_msg(run(task));
        report(&effects, outcome);
        observation(&effects, "request", Outcome::Succeeded, 1);
        assert!(
            !observation(&effects, "returned", outcome, returned)
                .metadata
                .truncated
        );
        if collection {
            observation(&effects, "collections.incomplete", Outcome::Partial, 1);
        }
        if entries != 0 {
            observation(&effects, "entries.incomplete", Outcome::Partial, entries);
        }
        assert_eq!(
            history(&effects).len(),
            3 + usize::from(collection) + usize::from(entries != 0)
        );
        assert_eq!(p.data.groups.len(), returned as usize);
        assert!(matches!(
            p.checks.get("groups"),
            Some((Check::Passed(_), _))
        ));
        assert_eq!(
            effects
                .iter()
                .filter(|fx| matches!(fx, Effect::Menu(_)))
                .count(),
            usize::from(returned != 0)
        );
        assert_eq!(recorded.requests().len(), 1);
    }
}

#[test]
fn group_failures_have_no_returned_or_fallback_claim_and_preserve_cache() {
    for reply in [
        ok(json!({"ok": false, "error": "PRIVATE_ERROR"})),
        Err(anyhow::anyhow!("PRIVATE_TRANSPORT")),
    ] {
        for pick in [false, true] {
            let reply = match &reply {
                Ok(value) => Ok(Reply {
                    status: value.status,
                    body: value.body.clone(),
                    ..Default::default()
                }),
                Err(error) => Err(anyhow::anyhow!(error.to_string())),
            };
            let (mut p, recorded) = sample(vec![reply]);
            let before = p.data.clone();
            let (_, task) = start(&mut p, pick);
            let effects = p.on_msg(run(task));
            report(&effects, Outcome::Failed);
            assert_eq!(
                observation(&effects, "request", Outcome::Failed, 1)
                    .metadata
                    .reason,
                Some(Reason::Unspecified)
            );
            assert_eq!(history(&effects).len(), 2);
            assert_eq!(p.data, before);
            assert!(!effects.iter().any(|fx| matches!(fx, Effect::Menu(_))));
            assert!(matches!(
                p.checks.get("groups"),
                Some((Check::Failed(_), _))
            ));
            assert_eq!(recorded.requests().len(), 1);
        }
    }
}

#[test]
fn retries_remain_one_logical_group_result() {
    let (mut p, recorded) = sample(vec![
        Ok(Reply {
            status: 429,
            retry_after: Some(0),
            ..Default::default()
        }),
        ok(body()),
    ]);
    let (_, task) = start(&mut p, false);
    let effects = p.on_msg(run(task));
    report(&effects, Outcome::Succeeded);
    observation(&effects, "request", Outcome::Succeeded, 1);
    let requests = recorded.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[test]
fn only_existing_template_cache_use_avoids_a_request_and_does_not_finish_pending_loads() {
    let (mut p, recorded) = sample(vec![ok(body())]);
    let (_, older) = start(&mut p, false);
    p.section_rows[2] = 2; // Review template.
    let before = (p.data.clone(), p.checks.clone(), p.sync.groups_at);
    let effects = settings::on_menu(&mut p, "review", "edit");
    assert_eq!(effects.len(), 1);
    observation(&effects, "cached", Outcome::Succeeded, 1);
    assert!(matches!(p.edit, Some((crate::Target::Template, _))));
    assert_eq!((p.data.clone(), p.checks.clone(), p.sync.groups_at), before);
    assert!(recorded.requests().is_empty());
    let effects = p.on_msg(run(older));
    report(&effects, Outcome::Succeeded);
    assert_eq!(
        p.data.groups.len(),
        2,
        "cache use does not supersede a pending request"
    );

    p.section_rows[2] = 1; // The group picker still refreshes a nonempty cache.
    let effects = settings::on_menu(&mut p, "review", "pick");
    assert!(
        effects
            .iter()
            .any(|fx| matches!(fx, Effect::SpawnAction { .. }))
    );
    assert!(history(&effects).is_empty());
    assert_eq!(
        recorded.requests().len(),
        1,
        "unpolled task adds no observations or requests"
    );
}

#[test]
fn empty_template_cache_dispatches_and_local_refusal_has_only_independent_evidence() {
    let (mut p, recorded) = sample(vec![]);
    p.data.groups.clear();
    p.section_rows[2] = 2;
    let effects = settings::on_menu(&mut p, "review", "edit");
    assert!(
        effects
            .iter()
            .any(|fx| matches!(fx, Effect::SpawnAction { .. }))
    );
    assert!(history(&effects).is_empty());
    assert!(matches!(p.edit, Some((crate::Target::Template, _))));
    p.via = Via(None);
    let effects = settings::fetch_groups(&mut p, true);
    assert_eq!(history(&effects).len(), 1);
    let refused = observation(&effects, "unavailable", Outcome::Blocked, 1);
    assert_eq!(refused.metadata.reason, Some(Reason::Unavailable));
    assert!(!effects.iter().any(|fx| matches!(
        fx,
        Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::Report(_)
    )));
    assert!(matches!(
        p.checks.get("groups"),
        Some((Check::Failed(_), _))
    ));
    assert!(recorded.requests().is_empty());
}

#[test]
fn cached_count_overflow_is_marked_without_an_inventory_claim() {
    if usize::BITS > 32 {
        let effects = [groups_cached(u32::MAX as usize + 1)];
        assert!(
            observation(&effects, "cached", Outcome::Succeeded, u32::MAX)
                .metadata
                .truncated
        );
    }
}

fn held(p: &SlackPlugin) -> String {
    format!(
        "{:?}",
        (
            &p.data,
            &p.cfg,
            &p.checks,
            p.sync.reading,
            p.sync.groups_at,
            p.sync.last_ok,
            &p.sync.errors,
            &p.edit
        )
    )
}

#[test]
fn every_rejected_group_result_preserves_actual_evidence_without_application() {
    for reject in ["token", "identity", "newer", "duplicate", "local_refusal"] {
        for outcome in [Outcome::Succeeded, Outcome::Partial, Outcome::Failed] {
            for pick in [false, true] {
                let reply = match outcome {
                    Outcome::Succeeded => ok(body()),
                    Outcome::Partial => ok(json!({"ok": true, "usergroups": [null]})),
                    _ => Err(anyhow::anyhow!("PRIVATE_FAILURE")),
                };
                let (mut p, recorded) = sample(vec![reply]);
                let (_, task) = start(&mut p, pick);
                let message = run(task).downcast::<SlackMsg>().ok().unwrap();
                let SlackMsg::Groups {
                    stamp,
                    action,
                    result,
                    pick,
                } = *message
                else {
                    panic!("group result")
                };
                let expected = format!("{:?}", groups(&action, &result));
                match reject {
                    "token" => p.replace_token("env:PRIVATE_NEXT".into()),
                    "identity" => {
                        let mut me = p.data.me.clone().unwrap();
                        me.team_id = "T0PRIVATE_NEXT".into();
                        p.install_me(me);
                    }
                    "newer" => {
                        p.begin_check("groups");
                    }
                    "duplicate" => assert!(p.complete_check("groups", stamp)),
                    "local_refusal" => {
                        p.via = Via(None);
                        settings::fetch_groups(&mut p, true);
                    }
                    _ => unreachable!(),
                }
                let before = held(&p);
                let effects = p.on_msg(Box::new(SlackMsg::Groups {
                    stamp,
                    action,
                    result,
                    pick,
                }));
                assert_eq!(held(&p), before, "{reject}");
                assert_eq!(format!("{:?}", &effects[..effects.len() - 1]), expected);
                report(&effects, outcome);
                assert!(
                    effects
                        .iter()
                        .all(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
                );
                let Some(Effect::Observe {
                    action,
                    outcome: Outcome::Blocked,
                }) = effects.last()
                else {
                    panic!("stale skip")
                };
                assert_eq!(action.kind.as_str(), "slack.read.apply.skipped.stale");
                assert_eq!(action.metadata.count, Some(1));
                assert!(action.subjects.is_empty());
                assert_eq!(recorded.requests().len(), 1);
            }
        }
    }
}

enum Command {
    Load,
    Cached,
    Refuse,
}
struct GroupHost(Arc<Mutex<SlackPlugin>>);
impl Plugin for GroupHost {
    fn id(&self) -> &'static str {
        "slack"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        let mut p = self.0.lock().unwrap();
        match message.downcast::<Command>() {
            Err(message) => p.on_msg(message),
            Ok(command) => {
                let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                    "test.enclosing.write",
                    Origin::Host,
                ))];
                effects.extend(match *command {
                    Command::Load => {
                        let mut fx = settings::fetch_groups(&mut p, false);
                        // Independent sibling reads keep their original scope.
                        fx.extend(settings::check_token(&mut p));
                        fx.extend(settings::on_bulk(&mut p, "ch-check-all"));
                        fx
                    }
                    Command::Cached => {
                        p.section_rows[2] = 2;
                        settings::on_menu(&mut p, "review", "edit")
                    }
                    Command::Refuse => {
                        p.via = Via(None);
                        settings::fetch_groups(&mut p, false)
                    }
                });
                effects.push(Effect::Report(Outcome::Succeeded));
                effects
            }
        }
    }
}

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
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

#[test]
fn host_group_outcomes_cannot_complete_enclosing_write_or_independent_siblings() {
    for (body, outcome) in [
        (
            json!({"ok": false, "error": "PRIVATE_ERROR"}),
            Outcome::Failed,
        ),
        (json!({"ok": true}), Outcome::Partial),
    ] {
        let (mut p, recorded) = sample(vec![
            ok(body),
            ok(json!({"ok": false, "error": "PRIVATE_AUTH_ERROR"})),
            ok(json!({"ok": true, "channel": {"id": "C001"}})),
        ]);
        let mut channel = demo::sample().cfg.channels.remove(0);
        channel.id = "C001".into();
        p.cfg.channels.push(channel);
        let mut host = Host::new(
            vec![Box::new(GroupHost(Arc::new(Mutex::new(p))))],
            demo::NOW,
        )
        .unwrap();
        host.enable_journal();
        let effects = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Command::Load),
        });
        let journal = records(&effects);
        let parent = id(&journal, "test.enclosing.write");
        let group = id(&journal, PREFIX);
        let auth = id(&journal, "slack.settings.check.workspace");
        let channel = id(&journal, "slack.settings.channel.resolve");
        assert_eq!(terminal(&journal, &parent), [Outcome::Succeeded]);
        assert!(terminal(&journal, &group).is_empty());
        let mut tasks = effects.into_iter().filter_map(|fx| match fx {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        });
        let (runtime, semantic, task) = tasks.next().unwrap();
        assert_eq!(semantic, Some(group.clone()));
        let siblings: Vec<_> = tasks.collect();
        assert_eq!(
            siblings.len(),
            2,
            "sibling checks did not acquire a group request"
        );
        assert_eq!(siblings[0].1, Some(auth.clone()));
        assert_eq!(siblings[1].1, Some(channel.clone()));
        let group_reply = run(task);
        for (sibling_runtime, sibling_semantic, sibling_task) in siblings {
            let expected = if sibling_semantic == Some(auth.clone()) {
                Outcome::Failed
            } else {
                Outcome::Partial
            };
            let sibling = sibling_semantic.clone().unwrap();
            let effects = host.update(Event::Tracked {
                action: sibling_runtime,
                semantic: sibling_semantic,
                event: Box::new(Event::Msg {
                    plugin: 0,
                    msg: run(sibling_task),
                }),
            });
            let journal = records(&effects);
            assert!(terminal(&journal, &group).is_empty());
            assert!(terminal(&journal, &runtime).is_empty());
            assert!(terminal(&journal, &parent).is_empty());
            assert_eq!(terminal(&journal, &sibling), [expected]);
            private_absent(&format!("{journal:?}"));
        }
        let effects = host.update(Event::Tracked {
            action: runtime.clone(),
            semantic,
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: group_reply,
            }),
        });
        let journal = records(&effects);
        assert_eq!(terminal(&journal, &group), [outcome]);
        assert_eq!(terminal(&journal, &runtime), [outcome]);
        assert!(terminal(&journal, &parent).is_empty());
        assert!(terminal(&journal, &auth).is_empty());
        assert!(terminal(&journal, &channel).is_empty());
        private_absent(&format!("{journal:?}"));
        assert_eq!(recorded.requests().len(), 3);
    }
}

#[test]
fn host_local_cache_and_refusal_observations_never_start_tasks_or_report_into_parent() {
    for (command, suffix, outcome) in [
        (Command::Cached, "cached", Outcome::Succeeded),
        (Command::Refuse, "unavailable", Outcome::Blocked),
    ] {
        let (p, recorded) = sample(vec![]);
        let mut host = Host::new(
            vec![Box::new(GroupHost(Arc::new(Mutex::new(p))))],
            demo::NOW,
        )
        .unwrap();
        host.enable_journal();
        let effects = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(command),
        });
        let journal = records(&effects);
        let parent = id(&journal, "test.enclosing.write");
        let observed = id(&journal, &format!("{PREFIX}.{suffix}"));
        assert_eq!(terminal(&journal, &parent), [Outcome::Succeeded]);
        assert_eq!(terminal(&journal, &observed), [outcome]);
        assert!(!journal.iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Started, .. } if id == &observed)));
        assert!(recorded.requests().is_empty());
        private_absent(&format!("{journal:?}"));
    }
}

#[test]
fn typed_combined_gaps_remain_separate_count_units() {
    let result = Ok(GroupsRead {
        groups: vec![],
        incomplete_collection: true,
        incomplete_entries: 3,
    });
    let effects = groups(&groups_action(), &result);
    report(&effects, Outcome::Partial);
    observation(&effects, "collections.incomplete", Outcome::Partial, 1);
    observation(&effects, "entries.incomplete", Outcome::Partial, 3);
}
