//! Recorded-only settings auth checks. No credentials or live provider calls.

use std::sync::{Arc, Mutex};

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, SubjectRole};
use radar_core::plugin::{Msg, Task};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

use super::*;
use crate::client::{Recorded, Reply};
use crate::{Check, SlackMsg, SlackPlugin, Via, demo, settings};

const PREFIX: &str = "slack.settings.check.workspace";
const TEAM: &str = "T000ACME1";
const NEXT_TEAM: &str = "T000NEXT1";
const USER: &str = "U0PRIVATEUSER99";

fn me(team: &str, user: &str) -> Me {
    Me {
        team_id: team.into(),
        user_id: user.into(),
        user: "PRIVATE_USERNAME".into(),
        team: "PRIVATE_TEAM_NAME".into(),
        url: "https://PRIVATE_WORKSPACE.invalid/".into(),
        scopes: vec!["PRIVATE_SCOPE".into()],
        tz_offset: None,
    }
}

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn auth(team: &str, user: &str) -> anyhow::Result<Reply> {
    ok(json!({"ok": true, "team_id": team, "user_id": user,
        "user": "PRIVATE_USERNAME", "team": "PRIVATE_TEAM_NAME",
        "url": "https://PRIVATE_WORKSPACE.invalid/"}))
}

fn sample(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let recorded = Recorded::new(replies);
    let mut p = SlackPlugin::new();
    p.via = Via(Some(recorded.clone()));
    assert!(p.install_me(me(TEAM, USER)));
    (p, recorded)
}

fn run(task: Task) -> Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}

fn start(p: &mut SlackPlugin) -> (ActionDescriptor, Task) {
    let effects = settings::check_token(p);
    let [Effect::SpawnAction { action, .. }] = effects.as_slice() else {
        panic!("one isolated settings auth task: {effects:?}");
    };
    assert!(
        action.subjects.is_empty(),
        "cached account is not authenticated scope"
    );
    assert_eq!(action.metadata.count, None);
    assert!(!action.metadata.redacted);
    match effects.into_iter().next().unwrap() {
        Effect::SpawnAction { action, task } => (action, task),
        _ => unreachable!(),
    }
}

fn history(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. }))
        .collect()
}

fn private_absent(text: &str) {
    for secret in ["PRIVATE_", USER, "env:", "xoxp-"] {
        assert!(
            !text.contains(secret),
            "history retained private content: {text}"
        );
    }
}

fn report(effects: &[Effect], expected: Outcome) {
    let got: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Report(outcome) => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(got, [expected]);
    private_absent(&format!("{:?}", history(effects)));
}

fn observed<'a>(effects: &'a [Effect], suffix: &str, expected: Outcome) -> &'a ActionDescriptor {
    let kind = format!("{PREFIX}.{suffix}");
    let found: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
                Some((action, outcome))
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "{effects:?}");
    assert_eq!(*found[0].1, expected);
    assert_eq!(found[0].0.metadata.count, Some(1));
    assert!(!found[0].0.metadata.truncated);
    found[0].0
}

#[test]
fn auth_success_is_distinct_from_complete_safe_identity() {
    for (team, user, complete, safe_team) in [
        (TEAM, USER, true, true),
        (TEAM, "W01", true, true),
        (TEAM, "", false, true),
        (TEAM, "U", false, true),
        (TEAM, "Ubad", false, true),
        (TEAM, "U1\n", false, true),
        ("", USER, false, false),
        ("T", USER, false, false),
        ("Tbad", USER, false, false),
        ("T1?PRIVATE_QUERY", USER, false, false),
        ("xoxp-PRIVATE_TOKEN", USER, false, false),
    ] {
        let outcome = if complete {
            Outcome::Succeeded
        } else {
            Outcome::Partial
        };
        let effects = workspace(&workspace_action(), &Ok(me(team, user)));
        report(&effects, outcome);
        assert!(
            observed(&effects, "auth", Outcome::Succeeded)
                .subjects
                .is_empty()
        );
        let identity = observed(&effects, "identity", outcome);
        assert_eq!(identity.metadata.redacted, !safe_team);
        assert_eq!(identity.metadata.omitted_subjects, u32::from(!safe_team));
        assert_eq!(identity.subjects.len(), usize::from(safe_team));
        for subject in &identity.subjects {
            assert_eq!(subject.role, SubjectRole::Result);
            assert_eq!(subject.key.provider, "slack");
            assert_eq!(subject.key.instance, TEAM);
            assert_eq!(subject.key.kind, "workspace");
            assert_eq!(subject.key.id, "workspace");
        }
    }
}

#[test]
fn identity_bounds_are_shared_with_bootstrap_and_never_keep_user_ids() {
    for length in [64, 2048, 2049] {
        let team = format!("T{}", "A".repeat(length - 1));
        let user = format!("W{}", "B".repeat(length - 1));
        for got in [me(&team, USER), me(TEAM, &user)] {
            let expected = if length <= 2048 {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            };
            let effects = workspace(&workspace_action(), &Ok(got));
            report(&effects, expected);
            assert!(!format!("{effects:?}").contains(&user));
        }
    }
}

#[test]
fn error_has_no_identity_evidence_or_cached_scope() {
    let effects = workspace(&workspace_action(), &Err("PRIVATE_PROVIDER_ERROR".into()));
    report(&effects, Outcome::Failed);
    assert_eq!(effects.len(), 2);
    let auth = observed(&effects, "auth", Outcome::Failed);
    assert!(auth.subjects.is_empty());
    assert_eq!(auth.metadata.reason, Some(Reason::Unspecified));
}

#[test]
fn local_refusal_has_no_task_started_or_auth_result_and_keeps_ui_error() {
    let mut p = SlackPlugin::new();
    assert!(p.install_me(me(TEAM, USER)));
    let effects = settings::check_token(&mut p);
    let refusal = observed(&effects, "unavailable", Outcome::Blocked);
    assert!(refusal.subjects.is_empty());
    assert_eq!(refusal.metadata.reason, Some(Reason::Unavailable));
    assert_eq!(history(&effects).len(), 1);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notify(_)))
    );
    assert!(matches!(
        p.checks.get("workspace"),
        Some((Check::Failed(_), _))
    ));
    private_absent(&format!("{:?}", history(&effects)));
}

#[test]
fn recorded_auth_preserves_retry_and_timezone_reuse_without_profile_request() {
    let (mut p, recorded) = sample(vec![
        Ok(Reply {
            status: 429,
            retry_after: Some(1),
            ..Default::default()
        }),
        auth(TEAM, USER),
    ]);
    p.cfg.token = "env:PRIVATE_TOKEN_REFERENCE".into();
    p.data.me.as_mut().unwrap().tz_offset = Some(3600);
    let (action, task) = start(&mut p);
    assert_eq!(action.kind.as_str(), PREFIX);
    assert!(recorded.requests().is_empty());
    let effects = p.on_msg(run(task));
    report(&effects, Outcome::Succeeded);
    observed(&effects, "auth", Outcome::Succeeded);
    observed(&effects, "identity", Outcome::Succeeded);
    assert_eq!(p.data.me.as_ref().unwrap().tz_offset, Some(3600));
    assert!(matches!(
        p.checks.get("workspace"),
        Some((Check::Passed(_), _))
    ));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert_eq!(recorded.requests().len(), 2);
    for request in recorded.requests() {
        assert_eq!(request.method, "auth.test");
        assert!(request.params.is_empty() && !request.write && request.body.is_none());
    }
    assert!(!p.snapshot().unwrap().to_string().contains(PREFIX));
}

#[test]
fn incomplete_auth_identity_is_reported_without_authority_or_follow_up_reads() {
    for body in [
        json!({"ok": true}),
        json!({"ok": true, "team_id": TEAM, "user_id": {"PRIVATE_PAYLOAD": true}}),
    ] {
        let (mut p, recorded) = sample(vec![ok(body)]);
        p.data.me.as_mut().unwrap().tz_offset = Some(3600);
        let (_, task) = start(&mut p);
        let effects = p.on_msg(run(task));
        report(&effects, Outcome::Partial);
        observed(&effects, "auth", Outcome::Succeeded);
        observed(&effects, "identity", Outcome::Partial);
        assert!(!p.authenticated());
        assert!(p.data.me.is_none());
        assert!(matches!(
            p.checks.get("workspace"),
            Some((Check::Failed(_), _))
        ));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
        );
        assert_eq!(
            recorded.requests().len(),
            1,
            "follow-up tasks are not executed"
        );
    }
}

#[test]
fn api_and_transport_errors_fail_without_new_requests_and_revoke_identity() {
    for reply in [
        ok(json!({"ok": false, "error": "PRIVATE_ERROR"})),
        Err(anyhow::anyhow!("PRIVATE_TRANSPORT_ERROR")),
    ] {
        let (mut p, recorded) = sample(vec![reply]);
        let (_, task) = start(&mut p);
        let effects = p.on_msg(run(task));
        report(&effects, Outcome::Failed);
        observed(&effects, "auth", Outcome::Failed);
        assert_eq!(history(&effects).len(), 2);
        assert!(p.data.me.is_none());
        assert!(!p.authenticated());
        assert!(matches!(
            p.checks.get("workspace"),
            Some((Check::Failed(_), _))
        ));
        assert_eq!(recorded.requests().len(), 1);
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
            p.sync.last_ok,
            &p.sync.errors
        )
    )
}

#[test]
fn every_rejected_auth_completion_keeps_actual_evidence_and_learned_scope() {
    for reject in ["token", "identity", "newer", "duplicate", "local_refusal"] {
        for failed in [false, true] {
            let reply = if failed {
                Err(anyhow::anyhow!("PRIVATE_ERROR"))
            } else {
                auth(TEAM, USER)
            };
            let (mut p, recorded) = sample(vec![reply]);
            let (_, task) = start(&mut p);
            let message = run(task).downcast::<SlackMsg>().ok().unwrap();
            let SlackMsg::WorkspaceChecked {
                stamp,
                action,
                result,
            } = *message
            else {
                panic!("auth result")
            };
            let expected = format!("{:?}", workspace(&action, &result));
            match reject {
                "token" => p.replace_token("env:PRIVATE_OTHER_REFERENCE".into()),
                "identity" => {
                    p.install_me(me(NEXT_TEAM, USER));
                }
                "newer" => {
                    p.begin_check("workspace");
                }
                "duplicate" => {
                    assert!(p.complete_check("workspace", stamp));
                }
                "local_refusal" => {
                    p.via = Via(None);
                    let effects = settings::check_token(&mut p);
                    observed(&effects, "unavailable", Outcome::Blocked);
                }
                _ => unreachable!(),
            }
            let before = held(&p);
            let effects = p.on_msg(Box::new(SlackMsg::WorkspaceChecked {
                stamp,
                action,
                result,
            }));
            assert_eq!(held(&p), before, "{reject}");
            assert_eq!(format!("{:?}", &effects[..effects.len() - 1]), expected);
            report(
                &effects,
                if failed {
                    Outcome::Failed
                } else {
                    Outcome::Succeeded
                },
            );
            assert!(
                effects
                    .iter()
                    .all(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. }))
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
            assert_eq!(action.subjects.len(), usize::from(!failed));
            if !failed {
                assert_eq!(action.subjects[0].key.instance, TEAM);
                assert_eq!(action.subjects[0].role, SubjectRole::Result);
            }
            assert_eq!(recorded.requests().len(), 1);
        }
    }
}

enum Command {
    Check,
    All,
    Refuse,
}
struct AuthHost(Arc<Mutex<SlackPlugin>>);
impl Plugin for AuthHost {
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
                    "test.enclosing",
                    Origin::Host,
                ))];
                effects.extend(match *command {
                    Command::Check => settings::check_token(&mut p),
                    Command::All => {
                        let mut reads = settings::check_token(&mut p);
                        reads.extend(settings::on_bulk(&mut p, "ch-check-all"));
                        reads
                    }
                    Command::Refuse => {
                        p.via = Via(None);
                        settings::check_token(&mut p)
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
fn independent_auth_cannot_complete_parent_or_capture_sibling_channel_tasks() {
    let (mut p, recorded) = sample(vec![
        Err(anyhow::anyhow!("PRIVATE_ERROR")),
        ok(json!({"ok": true, "channel": {"id": "C001", "name": "PRIVATE_CHANNEL_NAME"}})),
    ]);
    let mut channel = demo::sample().cfg.channels.remove(0);
    channel.id = "C001".into();
    channel.alias = "PRIVATE_ALIAS".into();
    p.cfg.channels.push(channel);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AuthHost(shared))], demo::NOW).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Command::All),
    });
    let journal = records(&effects);
    let parent = id(&journal, "test.enclosing");
    let auth = id(&journal, PREFIX);
    let channel = id(&journal, "slack.settings.channel.resolve");
    assert_ne!(channel, auth);
    assert_ne!(channel, parent);
    assert_eq!(terminal(&journal, &parent), [Outcome::Succeeded]);
    assert!(terminal(&journal, &auth).is_empty());
    let mut tasks = effects.into_iter().filter_map(|effect| match effect {
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
    assert_eq!(semantic, Some(auth.clone()));
    let (channel_runtime, channel_semantic, channel_task) = tasks.next().unwrap();
    assert_eq!(channel_semantic, Some(channel.clone()));
    assert!(tasks.next().is_none());
    let auth_message = run(task);
    let channel_effects = host.update(Event::Tracked {
        action: channel_runtime.clone(),
        semantic: channel_semantic,
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: run(channel_task),
        }),
    });
    let channel_journal = records(&channel_effects);
    assert!(terminal(&channel_journal, &auth).is_empty());
    assert!(terminal(&channel_journal, &runtime).is_empty());
    assert!(terminal(&channel_journal, &parent).is_empty());
    assert_eq!(terminal(&channel_journal, &channel), [Outcome::Partial]);
    assert_eq!(
        terminal(&channel_journal, &channel_runtime),
        [Outcome::Partial]
    );
    private_absent(&format!("{channel_journal:?}"));
    let effects = host.update(Event::Tracked {
        action: runtime.clone(),
        semantic,
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: auth_message,
        }),
    });
    let journal = records(&effects);
    assert_eq!(terminal(&journal, &auth), [Outcome::Failed]);
    assert_eq!(terminal(&journal, &runtime), [Outcome::Failed]);
    assert!(terminal(&journal, &parent).is_empty());
    assert!(terminal(&journal, &channel_runtime).is_empty());
    assert!(terminal(&journal, &channel).is_empty());
    private_absent(&format!("{journal:?}"));
    assert_eq!(recorded.requests().len(), 2);
}

#[test]
fn host_local_refusal_never_starts_or_reports_into_its_parent() {
    let (p, recorded) = sample(vec![]);
    let mut host = Host::new(vec![Box::new(AuthHost(Arc::new(Mutex::new(p))))], demo::NOW).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Command::Refuse),
    });
    let journal = records(&effects);
    let parent = id(&journal, "test.enclosing");
    let refusal = id(&journal, &format!("{PREFIX}.unavailable"));
    assert_eq!(terminal(&journal, &parent), [Outcome::Succeeded]);
    assert_eq!(terminal(&journal, &refusal), [Outcome::Blocked]);
    assert!(!journal.iter().any(|record| matches!(record,
        JournalRecord::Outcome { id, outcome: Outcome::Started, .. } if id == &refusal)));
    assert!(recorded.requests().is_empty());
    private_absent(&format!("{journal:?}"));
}

fn host_check(host: &mut Host) -> (ActionId, ActionId, Task) {
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Command::Check),
    });
    let journal = records(&effects);
    let auth = id(&journal, PREFIX);
    private_absent(&format!("{journal:?}"));
    assert!(terminal(&journal, &auth).is_empty());
    effects
        .into_iter()
        .find_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if semantic == auth => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        })
        .unwrap()
}

fn deliver(
    host: &mut Host,
    runtime: &ActionId,
    semantic: &ActionId,
    message: Msg,
) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: runtime.clone(),
        semantic: Some(semantic.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: message,
        }),
    })
}

#[test]
fn host_out_of_order_checks_keep_distinct_results_and_original_learned_scope() {
    let (p, recorded) = sample(vec![auth(NEXT_TEAM, ""), auth(TEAM, USER)]);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AuthHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let (old_runtime, old_auth, old_task) = host_check(&mut host);
    let old_message = run(old_task);
    let (new_runtime, new_auth, new_task) = host_check(&mut host);
    let new_effects = deliver(&mut host, &new_runtime, &new_auth, run(new_task));
    let journal = records(&new_effects);
    assert_eq!(terminal(&journal, &new_auth), [Outcome::Succeeded]);
    assert!(terminal(&journal, &old_auth).is_empty());
    let before = held(&shared.lock().unwrap());
    let old_effects = deliver(&mut host, &old_runtime, &old_auth, old_message);
    assert!(
        old_effects
            .iter()
            .all(|effect| matches!(effect, HostEffect::Journal(_)))
    );
    let journal = records(&old_effects);
    assert_eq!(terminal(&journal, &old_auth), [Outcome::Partial]);
    assert_eq!(terminal(&journal, &old_runtime), [Outcome::Partial]);
    assert!(terminal(&journal, &new_auth).is_empty());
    let skip = id(&journal, "slack.read.apply.skipped.stale");
    assert_eq!(terminal(&journal, &skip), [Outcome::Blocked]);
    for record in &journal {
        if let JournalRecord::Begin {
            descriptor, parent, ..
        } = record
        {
            assert_eq!(parent.as_ref(), Some(&old_runtime));
            if descriptor.kind.as_str() == format!("{PREFIX}.auth") {
                assert!(descriptor.subjects.is_empty());
            } else {
                assert_eq!(descriptor.subjects.len(), 1);
                assert_eq!(descriptor.subjects[0].role, SubjectRole::Result);
                assert_eq!(descriptor.subjects[0].key.instance, NEXT_TEAM);
            }
        }
    }
    assert_eq!(held(&shared.lock().unwrap()), before);
    assert_eq!(recorded.requests().len(), 2);
    private_absent(&format!("{journal:?}"));
}

#[test]
fn accepted_new_identity_starts_independent_follow_up_reads() {
    let (p, recorded) = sample(vec![
        auth(NEXT_TEAM, USER),
        auth(NEXT_TEAM, USER),
        Err(anyhow::anyhow!("PRIVATE_PROFILE_ERROR")),
    ]);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AuthHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let (runtime, auth, task) = host_check(&mut host);
    let effects = deliver(&mut host, &runtime, &auth, run(task));
    let journal = records(&effects);
    assert_eq!(terminal(&journal, &auth), [Outcome::Succeeded]);
    let bootstrap = id(&journal, "slack.sync.bootstrap");
    assert_eq!(
        shared.lock().unwrap().data.me.as_ref().unwrap().team_id,
        NEXT_TEAM
    );
    let followups: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                semantic, effect, ..
            } if matches!(**effect, HostEffect::Spawn { .. }) => Some(semantic),
            _ => None,
        })
        .collect();
    assert!(!followups.is_empty());
    assert!(
        followups
            .iter()
            .all(|semantic| semantic.is_some() && **semantic != Some(auth.clone()))
    );
    assert_eq!(
        recorded.requests().len(),
        1,
        "settings check itself made only its existing auth request"
    );
    private_absent(&format!("{journal:?}"));
    let (runtime, task) = effects
        .into_iter()
        .find_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if semantic == bootstrap => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, task)),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let effects = deliver(&mut host, &runtime, &bootstrap, run(task));
    let journal = records(&effects);
    assert_eq!(terminal(&journal, &bootstrap), [Outcome::Partial]);
    assert!(
        terminal(&journal, &auth).is_empty(),
        "profile fallback cannot relabel auth check success"
    );
    assert_eq!(
        recorded
            .requests()
            .iter()
            .map(|request| request.method)
            .collect::<Vec<_>>(),
        ["auth.test", "auth.test", "users.info"]
    );
    private_absent(&format!("{journal:?}"));
}
