//! Recorded-only regression coverage for reaction intent and completion ownership.

use std::sync::{Arc, Mutex};

use radar_core::Plugin;
use radar_core::journal::Outcome;
use radar_core::plugin::{Effect, Menu, Msg, Task};
use radar_core::writes::{self, Mode};
use serde_json::{Value, json};

use crate::client::{Recorded, Reply};
use crate::reaction_context::{ReactionOperation, ReactionPayload};
use crate::{SlackMsg, SlackPlugin, Via, Write, demo, page, thread_key};

const ROOT: &str = "1791200000.000100";
const MESSAGE: &str = "1791200001.000100";
const OTHER_ROOT: &str = "1791200002.000100";
const ERROR: &str = "private_reaction_error_canary";

struct ModeGuard((Mode, Option<String>));
impl ModeGuard {
    fn edit() -> Self {
        let previous = writes::mode();
        writes::set_mode(Mode::Edit, None);
        Self(previous)
    }
}
impl Drop for ModeGuard {
    fn drop(&mut self) {
        writes::set_mode(self.0.0, self.0.1.clone());
    }
}

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}

fn sample(failed: bool) -> (SlackPlugin, Arc<Recorded>, ModeGuard) {
    let mode = ModeGuard::edit();
    let mut p = demo::sample();
    let recorded = Recorded::new(vec![
        ok(if failed {
            json!({"ok": false, "error": ERROR})
        } else {
            json!({"ok": true})
        }),
        ok(json!({"ok": true, "messages": []})),
    ]);
    p.via = Via(Some(recorded.clone()));
    p.data.channels.get_mut(demo::TEAM).unwrap().replies.insert(
        ROOT.into(),
        vec![demo::msg(
            MESSAGE,
            demo::BOB,
            "private_reaction_message_canary",
        )],
    );
    (p, recorded, mode)
}

fn menu(effects: Vec<Effect>) -> Menu {
    effects
        .into_iter()
        .find_map(|fx| match fx {
            Effect::Menu(menu) => Some(menu),
            _ => None,
        })
        .expect("reaction menu")
}
fn pick(p: &mut SlackPlugin) -> String {
    menu(page::react_menu(p, demo::TEAM, MESSAGE)).id
}
fn preview(p: &mut SlackPlugin) -> String {
    let picker = pick(p);
    menu(p.answer_reaction(&picker, "eyes").unwrap()).id
}
fn tasks(effects: Vec<Effect>) -> Vec<Task> {
    effects
        .into_iter()
        .filter_map(|fx| match fx {
            Effect::Spawn(task) | Effect::SpawnAction { task, .. } => Some(task),
            _ => None,
        })
        .collect()
}
fn run(task: Task) -> SlackMsg {
    *tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
        .downcast::<SlackMsg>()
        .expect("Slack result")
}
fn send(p: &mut SlackPlugin, confirmation: &str) -> SlackMsg {
    let mut work = tasks(p.answer_reaction(confirmation, "yes").unwrap());
    assert_eq!(work.len(), 1);
    run(work.remove(0))
}
fn duplicate(message: &SlackMsg) -> SlackMsg {
    let SlackMsg::Wrote {
        instance,
        what,
        result,
    } = message
    else {
        panic!("reaction result")
    };
    SlackMsg::Wrote {
        instance: instance.clone(),
        what: what.clone(),
        result: result.clone(),
    }
}
fn held(p: &SlackPlugin) -> String {
    format!(
        "{:?}",
        (
            &p.data,
            &p.cfg,
            &p.pages,
            &p.threads,
            &p.replying,
            &p.reply_context,
            &p.draft,
            &p.ask_context,
            &p.checks,
            &p.sync
        )
    )
}
fn no_dispatch(effects: Vec<Effect>) {
    assert!(
        !effects.iter().any(|fx| matches!(
            fx,
            Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::PersistCache
        )),
        "{effects:?}"
    );
}
fn evidence_only(effects: &[Effect], expected: Outcome, observation: &str) {
    assert!(matches!(effects.first(), Some(Effect::Report(outcome)) if *outcome == expected));
    assert!(
        effects
            .iter()
            .all(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
    );
    assert!(effects.iter().any(|fx| matches!(fx, Effect::Observe { action, outcome: Outcome::Blocked }
        if action.kind.as_str() == observation && action.metadata.count == Some(1) && action.subjects.is_empty())));
}
fn change_account(p: &mut SlackPlugin, change: &str) {
    let token = p.cfg.token.clone();
    let original = p.data.me.clone().unwrap();
    if change.starts_with("token") {
        p.replace_token("env:OTHER_SLACK_TOKEN".into());
        if change.ends_with("aba") {
            p.replace_token(token);
            p.install_me(original);
        }
    } else {
        let mut changed = original.clone();
        if change.starts_with("user") {
            changed.user_id = "U000OTHER1".into();
        } else {
            changed.team_id = "T000OTHER1".into();
        }
        p.install_me(changed);
        if change.ends_with("aba") {
            p.install_me(original);
        }
    }
}

#[test]
fn accepted_reaction_freezes_payload_and_refresh_root_without_post_result() {
    let (mut p, recorded, _mode) = sample(false);
    let picker = pick(&mut p);
    // Even changes before selecting an emoji must not retarget its scope/refresh.
    p.pages.push(page::Page::Thread {
        channel: demo::TEAM.into(),
        ts: OTHER_ROOT.into(),
        tab: 0,
        row: 0,
    });
    let replies = &mut p.data.channels.get_mut(demo::TEAM).unwrap().replies;
    let messages = replies.remove(ROOT).unwrap();
    replies.insert(OTHER_ROOT.into(), messages);
    let effects = p.answer_reaction(&picker, "eyes").unwrap();
    assert!(effects.iter().any(|fx| matches!(fx, Effect::Journal(action)
        if action.subjects.iter().any(|subject| subject.key.kind == "thread"
            && subject.key.id == format!("{}/{ROOT}", demo::TEAM)))));
    let id = menu(effects).id;
    let message = send(&mut p, &id);
    let effects = p.on_msg(Box::new(message));
    assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
    assert!(!effects.iter().any(|fx| matches!(fx, Effect::Journal(_))));
    let mut refresh = tasks(effects);
    assert_eq!(refresh.len(), 1);
    p.on_msg(Box::new(run(refresh.remove(0))));
    let requests = recorded.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "reactions.add");
    assert_eq!(
        requests[0].body,
        Some(json!({"channel": demo::TEAM, "timestamp": MESSAGE, "name": "eyes"}))
    );
    assert_eq!(requests[1].method, "conversations.replies");
    assert!(
        requests[1]
            .params
            .iter()
            .any(|(key, value)| key == "ts" && value == ROOT)
    );
}

#[test]
fn picker_and_confirmation_reject_account_changes_including_return_to_original_identity() {
    for (stage, answer) in [("pick", "eyes"), ("confirm", "yes"), ("confirm", "no")] {
        for change in ["token", "token-aba", "team", "team-aba", "user", "user-aba"] {
            let (mut p, recorded, _mode) = sample(false);
            let id = if stage == "pick" {
                pick(&mut p)
            } else {
                preview(&mut p)
            };
            change_account(&mut p, change);
            let before = held(&p);
            let effects = p.answer_reaction(&id, answer).unwrap();
            assert!(matches!(effects[0], Effect::Report(Outcome::Blocked)));
            assert!(!effects.iter().any(|fx| matches!(fx, Effect::Menu(_))));
            no_dispatch(effects);
            assert_eq!(held(&p), before, "{stage}/{change}");
            assert!(recorded.requests().is_empty());
        }
    }
}

#[test]
fn stale_answers_cannot_consume_newer_picker_or_confirmation_and_all_choices_are_one_use() {
    let (mut p, recorded, _mode) = sample(false);
    let old = pick(&mut p);
    let latest = pick(&mut p);
    assert_ne!(old, latest);
    no_dispatch(p.answer_reaction(&old, "eyes").unwrap());
    let first = menu(p.answer_reaction(&latest, "eyes").unwrap()).id;
    no_dispatch(p.answer_reaction(&latest, "eyes").unwrap());
    let second = preview(&mut p);
    assert_ne!(first, second);
    for answer in ["yes", "no", "edit"] {
        no_dispatch(p.answer_reaction(&first, answer).unwrap());
    }
    let _result = send(&mut p, &second);
    for answer in ["yes", "no", "edit"] {
        no_dispatch(p.answer_reaction(&second, answer).unwrap());
    }
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn invalid_emoji_and_decline_consume_their_choices_without_sending() {
    let (mut p, recorded, _mode) = sample(false);
    let picker = pick(&mut p);
    no_dispatch(
        p.answer_reaction(&picker, "arbitrary_unoffered_emoji")
            .unwrap(),
    );
    let refused = p.answer_reaction(&picker, "eyes").unwrap();
    assert!(!refused.iter().any(|fx| matches!(fx, Effect::Menu(_))));
    let confirm = preview(&mut p);
    let declined = p.answer_reaction(&confirm, "no").unwrap();
    assert!(matches!(declined[0], Effect::Report(Outcome::Cancelled)));
    no_dispatch(p.answer_reaction(&confirm, "yes").unwrap());
    assert!(recorded.requests().is_empty());
}

#[test]
fn unchanged_token_reference_and_navigation_do_not_invalidate_the_picker() {
    let (mut p, recorded, _mode) = sample(false);
    let picker = pick(&mut p);
    p.replace_token(p.cfg.token.clone());
    p.pages.clear();
    let confirmation = menu(p.answer_reaction(&picker, "eyes").unwrap()).id;
    let _message = send(&mut p, &confirmation);
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn local_client_and_mode_refusals_retire_confirmation_before_recovery() {
    for client_failure in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let confirmation = preview(&mut p);
        let token = p.cfg.token.clone();
        // A bad client still fails locally before the read-only mode refusal.
        writes::set_mode(Mode::ReadOnly, None);
        if client_failure {
            p.via = Via(None);
            p.cfg.token.clear();
        }
        let effects = p.answer_reaction(&confirmation, "yes").unwrap();
        assert!(effects.iter().any(|fx| matches!(fx, Effect::Report(outcome)
            if *outcome == if client_failure { Outcome::Failed } else { Outcome::Blocked })));
        no_dispatch(effects);
        assert!(format!("{:?}", p.reaction_context).contains("active: {}"));
        p.cfg.token = token;
        p.via = Via(Some(recorded.clone()));
        writes::set_mode(Mode::Edit, None);
        no_dispatch(p.answer_reaction(&confirmation, "yes").unwrap());
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn duplicate_dispatch_and_completed_results_cannot_send_notify_or_refresh_again() {
    for failed in [false, true] {
        let (mut p, recorded, _mode) = sample(failed);
        let confirmation = preview(&mut p);
        let message = send(&mut p, &confirmation);
        let repeated = duplicate(&message);
        let SlackMsg::Wrote { what, .. } = &message else {
            unreachable!()
        };
        no_dispatch(p.write(what.clone()));
        let effects = p.on_msg(Box::new(message));
        for task in tasks(effects) {
            p.on_msg(Box::new(run(task)));
        }
        let next = preview(&mut p);
        let before = held(&p);
        let effects = p.on_msg(Box::new(repeated));
        evidence_only(
            &effects,
            if failed {
                Outcome::Unknown
            } else {
                Outcome::Succeeded
            },
            "slack.reaction.completion.duplicate",
        );
        assert_eq!(held(&p), before);
        assert_eq!(recorded.requests().len(), if failed { 1 } else { 2 });
        assert!(matches!(
            p.answer_reaction(&next, "no").unwrap()[0],
            Effect::Report(Outcome::Cancelled)
        ));
    }
}

#[test]
fn stale_completed_reactions_preserve_actual_outcomes_without_touching_current_context() {
    for failed in [false, true] {
        for change in ["token", "token-aba", "team", "team-aba", "user", "user-aba"] {
            let (mut p, recorded, _mode) = sample(failed);
            let confirmation = preview(&mut p);
            let message = send(&mut p, &confirmation);
            change_account(&mut p, change);
            p.threads
                .entry(thread_key(demo::TEAM, ROOT))
                .or_default()
                .error = Some("current thread error".into());
            p.threads
                .entry(thread_key(demo::TEAM, OTHER_ROOT))
                .or_default()
                .loading = true;
            p.begin_reply(demo::TEAM, OTHER_ROOT, Some("new reply text"));
            let next = if p.authenticated() {
                Some(preview(&mut p))
            } else {
                let blocked = page::react_menu(&mut p, demo::TEAM, MESSAGE);
                assert!(
                    !blocked
                        .iter()
                        .any(|effect| matches!(effect, Effect::Menu(_)))
                );
                no_dispatch(blocked);
                None
            };
            let before = held(&p);
            let effects = p.on_msg(Box::new(message));
            assert!(
                matches!(effects[0], Effect::Report(outcome) if outcome == if failed { Outcome::Unknown } else { Outcome::Succeeded })
            );
            assert!(effects.iter().any(|fx| matches!(fx, Effect::Observe { action, .. } if action.kind.as_str() == "slack.reaction.apply.skipped.stale")));
            no_dispatch(effects);
            assert_eq!(held(&p), before, "{failed}/{change}");
            assert_eq!(recorded.requests().len(), 1);
            if let Some(next) = next {
                assert!(matches!(
                    p.answer_reaction(&next, "no").unwrap()[0],
                    Effect::Report(Outcome::Cancelled)
                ));
            }
        }
    }
}

#[test]
fn unmatched_result_or_operation_cannot_consume_the_real_active_send() {
    let (mut p, recorded, _mode) = sample(false);
    let confirmation = preview(&mut p);
    let message = send(&mut p, &confirmation);
    for wrong in ["id", "payload", "read"] {
        let mut unmatched = duplicate(&message);
        let SlackMsg::Wrote {
            what: Write::React { operation },
            ..
        } = &mut unmatched
        else {
            unreachable!()
        };
        match wrong {
            "id" => operation.id += 100,
            "payload" => operation.payload.name = "+1".into(),
            "read" => {
                let mut other = demo::sample();
                other.replace_token("env:OTHER_SLACK_TOKEN".into());
                operation.read = other.read_stamp();
            }
            _ => unreachable!(),
        }
        let before = held(&p);
        let effects = p.on_msg(Box::new(unmatched));
        evidence_only(
            &effects,
            Outcome::Succeeded,
            "slack.reaction.completion.unmatched",
        );
        assert_eq!(held(&p), before);
    }
    assert_eq!(tasks(p.on_msg(Box::new(message))).len(), 1);
    assert_eq!(recorded.requests().len(), 1);
    let unregistered = Write::React {
        operation: ReactionOperation {
            id: 0,
            read: p.read_stamp(),
            payload: ReactionPayload {
                channel: demo::TEAM.into(),
                ts: MESSAGE.into(),
                name: "eyes".into(),
                thread: ROOT.into(),
            },
        },
    };
    no_dispatch(p.write(unregistered.clone()));
    let effects = p.on_wrote(unregistered, Err(ERROR.into()), "T000ACME1");
    evidence_only(
        &effects,
        Outcome::Unknown,
        "slack.reaction.completion.unmatched",
    );
}

struct OpenPicker;
struct ReactionHost(Arc<Mutex<SlackPlugin>>);
impl Plugin for ReactionHost {
    fn id(&self) -> &'static str {
        "slack"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let mut p = self.0.lock().unwrap();
        match msg.downcast::<OpenPicker>() {
            Ok(_) => page::react_menu(&mut p, demo::TEAM, MESSAGE),
            Err(msg) => p.on_msg(msg),
        }
    }
    fn on_menu(&mut self, id: &str, value: &str) -> Vec<Effect> {
        page::on_menu(&mut self.0.lock().unwrap(), id, value).unwrap_or_default()
    }
}

#[test]
fn host_escape_reopen_and_stale_success_preserve_original_scope_without_private_history() {
    use radar_core::host::{Event, HostEffect, JournalRecord};
    use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let (p, recorded, _mode) = sample(false);
    let shared = Arc::new(Mutex::new(p));
    let mut host =
        radar_core::Host::new(vec![Box::new(ReactionHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(OpenPicker),
    });
    let old_picker = host.menu.as_ref().unwrap().menu.id.clone();
    host.update(key(KeyCode::Esc));
    host.update(key(KeyCode::Char('y')));
    assert!(host.menu.is_none());
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(OpenPicker),
    });
    assert_ne!(host.menu.as_ref().unwrap().menu.id, old_picker);
    host.update(key(KeyCode::Char('e')));
    let old_confirm = host.menu.as_ref().unwrap().menu.id.clone();
    host.update(key(KeyCode::Esc));
    host.update(key(KeyCode::Char('y')));
    assert!(recorded.requests().is_empty());
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(OpenPicker),
    });
    let preview = host.update(key(KeyCode::Char('e')));
    assert_ne!(host.menu.as_ref().unwrap().menu.id, old_confirm);
    let scope = preview
        .iter()
        .find_map(|fx| match fx {
            HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                if descriptor.kind.as_str() == "slack.reaction.add" =>
            {
                assert!(
                    descriptor
                        .subjects
                        .iter()
                        .all(|s| s.key.instance == "T000ACME1")
                );
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    let dispatched = host.update(key(KeyCode::Char('y')));
    let (action, task) = dispatched
        .into_iter()
        .find_map(|fx| match fx {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if semantic == scope => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, task)),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let message = run(task);
    change_account(&mut shared.lock().unwrap(), "team");
    let before = held(&shared.lock().unwrap());
    let results = host.update(Event::Tracked {
        action: action.clone(),
        semantic: Some(scope.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(message),
        }),
    });
    assert_eq!(held(&shared.lock().unwrap()), before);
    assert_eq!(recorded.requests().len(), 1);
    assert!(
        !results
            .iter()
            .any(|fx| matches!(fx, HostEffect::Tracked { .. } | HostEffect::Spawn { .. }))
    );
    let records: Vec<_> = preview
        .iter()
        .chain(results.iter())
        .filter_map(|fx| match fx {
            HostEffect::Journal(record) => Some(record),
            _ => None,
        })
        .collect();
    for target in [&scope, &action] {
        let terminal: Vec<_> = records
            .iter()
            .filter_map(|r| match r {
                JournalRecord::Outcome { id, outcome, .. }
                    if id == target && outcome.is_terminal() =>
                {
                    Some(*outcome)
                }
                _ => None,
            })
            .collect();
        assert_eq!(terminal, [Outcome::Succeeded]);
    }
    let history = format!("{records:?}");
    assert!(!history.contains("slack.message.post.result"));
    assert!(
        !history.contains("private_reaction")
            && !history.contains("eyes")
            && !history.contains("T000OTHER1")
    );
}

#[test]
fn reaction_choices_and_operations_are_transient_and_history_excludes_presentation_and_errors() {
    let (mut p, recorded, _mode) = sample(true);
    let snapshot = p.snapshot();
    let picker = menu(p.pick_reaction(
        demo::TEAM,
        MESSAGE,
        "private_reaction_presentation_canary".into(),
    ))
    .id;
    assert_eq!(p.snapshot(), snapshot);
    let preview = p.answer_reaction(&picker, "eyes").unwrap();
    let history: Vec<_> = preview
        .iter()
        .filter(|effect| matches!(effect, Effect::Journal(_) | Effect::Observe { .. }))
        .collect();
    let history = format!("{history:?}");
    assert!(!history.contains("private_reaction") && !history.contains("eyes"));
    let confirmation = menu(preview).id;
    assert_eq!(p.snapshot(), snapshot);
    let result = send(&mut p, &confirmation);
    assert_eq!(p.snapshot(), snapshot);
    let effects = p.on_msg(Box::new(result));
    assert!(matches!(effects[0], Effect::Report(Outcome::Unknown)));
    let history: Vec<_> = effects
        .iter()
        .filter(|effect| matches!(effect, Effect::Journal(_) | Effect::Observe { .. }))
        .collect();
    assert!(!format!("{history:?}").contains("private_reaction"));
    no_dispatch(effects);
    assert_eq!(p.snapshot(), snapshot);
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn unverified_cached_identity_cannot_choose_or_confirm_a_reaction() {
    let (mut p, recorded, _mode) = sample(false);
    let confirmation = preview(&mut p);
    p.session_owner = None;
    let effects = p.answer_reaction(&confirmation, "yes").unwrap();
    assert!(tasks(effects).is_empty());
    assert!(
        !p.pick_reaction(demo::TEAM, MESSAGE, "earlier author".into())
            .iter()
            .any(|effect| matches!(effect, Effect::Menu(_)))
    );
    assert!(recorded.requests().is_empty());
}
