//! Reply confirmations, writes and recovery use synthetic transports only.
//! Delayed results must preserve every newer composer and saved draft.

mod menu_navigation;

use std::sync::Arc;

use radar_core::Plugin;
use radar_core::host::{Event, HostEffect, OpenMenu, Owner, View};
use radar_core::input::TextInput;
use radar_core::journal::Outcome as JournalOutcome;
use radar_core::plugin::{Effect, Outcome, Pane, Task};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::writes::{self, Mode};
use serde_json::{Value, json};

use crate::ask_context::DraftStamp;
use crate::client::{Recorded, Reply};
use crate::reply_context::{MAX_KEPT_BYTES, MAX_KEPT_REPLIES, ReplyStatus};
use crate::{SlackMsg, SlackPlugin, Via, demo};

const ROOT: &str = "1791200000.000100";
const OTHER_ROOT: &str = "1791200001.000100";
const POSTED: &str = "1791271000.000900";
const TEXT: &str = "private_reply_words_canary\nsecond line  \n";

struct ModeGuard((Mode, Option<String>));

impl ModeGuard {
    fn edit() -> Self {
        let before = writes::mode();
        writes::set_mode(Mode::Edit, None);
        Self(before)
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
            json!({"ok": false, "error": "private_reply_error_canary"})
        } else {
            json!({"ok": true, "channel": demo::TEAM, "ts": POSTED})
        }),
        ok(json!({"ok": true, "messages": []})),
    ]);
    p.via = Via(Some(recorded.clone()));
    p.begin_reply(demo::TEAM, ROOT, Some(TEXT));
    (p, recorded, mode)
}

fn menu_id(effects: Vec<Effect>, prefix: &str) -> String {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Menu(menu) => {
                assert!(menu.id.starts_with(prefix), "{}", menu.id);
                Some(menu.id)
            }
            _ => None,
        })
        .expect("reply menu")
}

fn menu_contains(effects: &[Effect], text: &str) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Menu(menu) if menu.items.iter().any(|item| item.label.contains(text)))
    })
}

fn preview(p: &mut SlackPlugin) -> String {
    menu_id(p.preview_reply(), "reply-send:")
}

fn tasks(effects: Vec<Effect>) -> Vec<Task> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Spawn(task) | Effect::SpawnAction { task, .. } => Some(task),
            _ => None,
        })
        .collect()
}

fn run(task: Task) -> SlackMsg {
    let message = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task);
    *message.downcast::<SlackMsg>().expect("Slack result")
}

fn send(p: &mut SlackPlugin, menu: &str) -> SlackMsg {
    let mut started = tasks(p.answer_reply(menu, "yes").expect("reply answer"));
    assert_eq!(started.len(), 1, "one task per accepted reply");
    run(started.remove(0))
}

fn duplicate(message: &SlackMsg) -> SlackMsg {
    let SlackMsg::Wrote {
        instance,
        what,
        result,
    } = message
    else {
        panic!("write result")
    };
    SlackMsg::Wrote {
        instance: instance.clone(),
        what: what.clone(),
        result: result.clone(),
    }
}

fn current(p: &SlackPlugin) -> Option<(DraftStamp, String, String, String)> {
    p.replying.as_ref().map(|reply| {
        (
            reply.stamp,
            reply.channel.clone(),
            reply.thread.clone(),
            reply.composer.text(),
        )
    })
}

fn kept(p: &SlackPlugin) -> Value {
    Value::Array(
        p.reply_context
            .kept
            .values()
            .map(|reply| {
                json!({
                    "id": reply.id,
                    "channel": reply.channel,
                    "thread": reply.thread,
                    "original": reply.original,
                    "team": reply.team,
                    "read": format!("{:?}", reply.read),
                    "status": format!("{:?}", reply.status),
                })
            })
            .collect(),
    )
}

fn kept_texts(p: &SlackPlugin) -> Vec<String> {
    p.reply_context
        .kept
        .values()
        .map(|reply| reply.original.clone())
        .collect()
}

fn key(p: &mut SlackPlugin, code: KeyCode) {
    let _ = crate::page::reply_key(p, KeyEvent::new(code, KeyModifiers::NONE));
}

fn assert_no_send(effects: Option<Vec<Effect>>, recorded: &Recorded) {
    assert!(tasks(effects.unwrap_or_default()).is_empty());
    assert!(recorded.requests().is_empty());
}

fn discard_current(p: &mut SlackPlugin) {
    let menu = menu_id(p.preview_reply_drop(), "reply-drop:");
    p.answer_reply(&menu, "yes").expect("drop current reply");
    assert!(p.replying.is_none());
}

fn keep(p: &mut SlackPlugin, thread: &str, text: &str) {
    p.begin_reply(demo::TEAM, thread, Some(text));
    assert_eq!(p.replying.as_ref().unwrap().composer.text(), text);
    let menu = preview(p);
    p.answer_reply(&menu, "no").expect("keep reply");
    assert!(p.replying.is_none());
}

fn entry(p: &mut SlackPlugin, id: u64) -> String {
    let list = menu_id(p.kept_replies(), "reply-kept:");
    menu_id(
        p.answer_reply(&list, &format!("entry:{id}"))
            .expect("retained reply choice"),
        "reply-kept-entry:",
    )
}

fn pending_choice(effects: Vec<Effect>) -> (String, String) {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Menu(menu) => {
                assert!(menu.id.starts_with("reply-kept:"));
                let value = menu
                    .items
                    .iter()
                    .find(|item| item.value.starts_with("active:"))?
                    .value
                    .clone();
                Some((menu.id, value))
            }
            _ => None,
        })
        .expect("pending operation listed")
}

#[test]
fn accepted_reply_sends_exact_preview_once_and_refreshes_the_current_thread() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    assert!(p.replying.is_none());
    let requests = recorded.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "chat.postMessage");
    assert!(requests[0].write);
    assert_eq!(requests[0].body.as_ref().unwrap()["channel"], demo::TEAM);
    assert_eq!(requests[0].body.as_ref().unwrap()["thread_ts"], ROOT);
    assert_eq!(requests[0].body.as_ref().unwrap()["text"], TEXT.trim_end());

    let effects = p.on_msg(Box::new(message));
    assert!(matches!(
        effects.first(),
        Some(Effect::Report(JournalOutcome::Succeeded))
    ));
    let mut refresh = tasks(effects);
    assert_eq!(refresh.len(), 1);
    let result = run(refresh.remove(0));
    p.on_msg(Box::new(result));
    assert_eq!(recorded.requests().len(), 2);
    assert_eq!(recorded.requests()[1].method, "conversations.replies");
}

#[test]
fn old_menu_cannot_consume_a_new_preview_and_confirmation_is_single_use() {
    let (mut p, recorded, _mode) = sample(false);
    let old = preview(&mut p);
    let latest = preview(&mut p);
    assert_ne!(old, latest);
    let before = current(&p);
    for value in ["yes", "edit", "no"] {
        assert_no_send(p.answer_reply(&old, value), &recorded);
        assert_eq!(current(&p), before);
    }
    let _message = send(&mut p, &latest);
    assert!(tasks(p.answer_reply(&latest, "yes").unwrap_or_default()).is_empty());
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn text_paste_target_or_account_changes_make_every_old_answer_harmless() {
    for mutation in ["typing", "paste", "target", "token", "identity"] {
        for answer in ["yes", "edit", "no"] {
            let (mut p, recorded, _mode) = sample(false);
            let old = preview(&mut p);
            match mutation {
                "typing" => key(&mut p, KeyCode::Char('x')),
                "paste" => {
                    p.paste("newer text");
                }
                "target" => {
                    p.begin_reply(demo::TEAM, OTHER_ROOT, Some("new target text"));
                }
                "token" => p.replace_token("env:OTHER_SLACK_TOKEN".into()),
                "identity" => {
                    let mut me = p.data.me.clone().unwrap();
                    me.team_id = "T000OTHER1".into();
                    p.install_me(me);
                }
                _ => unreachable!(),
            }
            let before = (current(&p), kept(&p));
            assert_no_send(p.answer_reply(&old, answer), &recorded);
            assert_eq!((current(&p), kept(&p)), before, "{mutation}/{answer}");
        }
    }
}

#[test]
fn stale_drop_does_not_discard_new_text_or_a_recreated_composer() {
    for replacement in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let old = menu_id(p.preview_reply_drop(), "reply-drop:");
        if replacement {
            let current_drop = menu_id(p.preview_reply_drop(), "reply-drop:");
            p.answer_reply(&current_drop, "yes").expect("drop current");
            assert!(p.replying.is_none());
            p.begin_reply(demo::TEAM, ROOT, Some(TEXT));
        } else {
            p.paste("more work");
        }
        let before = (current(&p), kept(&p));
        assert_no_send(p.answer_reply(&old, "yes"), &recorded);
        assert_eq!((current(&p), kept(&p)), before);
    }
}

#[test]
fn same_target_reopen_and_reask_seed_preserve_existing_and_kept_text() {
    let (mut p, recorded, _mode) = sample(false);
    let before = current(&p);
    p.begin_reply(demo::TEAM, ROOT, None);
    assert_eq!(current(&p), before);
    p.begin_reply(demo::TEAM, ROOT, Some("generated reask words"));
    assert_eq!(
        p.replying.as_ref().unwrap().composer.text(),
        "generated reask words"
    );
    assert_eq!(kept_texts(&p), [TEXT]);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("keep declined reply");
    assert!(p.replying.is_none());
    let before = kept(&p);
    p.begin_reply(demo::TEAM, ROOT, Some("generated reask words"));
    assert_eq!(kept(&p), before);
    assert_eq!(kept_texts(&p), [TEXT, "generated reask words"]);
    assert!(recorded.requests().is_empty());
}

#[test]
fn revisions_follow_actual_multiline_edits_and_paste_but_not_cursor_moves() {
    let (mut p, _, _mode) = sample(false);
    let initial = p.replying.as_ref().unwrap().stamp;
    key(&mut p, KeyCode::Home);
    key(&mut p, KeyCode::Left);
    assert_eq!(p.replying.as_ref().unwrap().stamp, initial);
    p.paste("");
    assert_eq!(p.replying.as_ref().unwrap().stamp, initial);
    key(&mut p, KeyCode::Enter);
    let newline = p.replying.as_ref().unwrap().stamp;
    assert_ne!(newline, initial);
    key(&mut p, KeyCode::Backspace);
    let joined = p.replying.as_ref().unwrap().stamp;
    assert_ne!(joined, newline);
    p.paste("new\nlines");
    assert_ne!(p.replying.as_ref().unwrap().stamp, joined);
}

#[test]
fn local_client_failure_and_mode_refusal_preserve_untrimmed_reply_text() {
    for local_failure in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        if local_failure {
            p.via = Via(None);
            p.cfg.token.clear();
        } else {
            writes::set_mode(Mode::ReadOnly, None);
        }
        let effects = p.answer_reply(&menu, "yes").expect("reply answer");
        assert!(tasks(effects).is_empty());
        assert!(recorded.requests().is_empty());
        assert_eq!(kept_texts(&p), [TEXT]);
        assert_eq!(p.reply_context.usage(), (1, TEXT.len()));
    }
}

#[test]
fn duplicate_completion_never_restores_over_newer_work_or_refreshes_twice() {
    for failed in [false, true] {
        let (mut p, recorded, _mode) = sample(failed);
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        let repeated = duplicate(&message);
        p.on_msg(Box::new(message));
        p.begin_reply(demo::TEAM, OTHER_ROOT, Some("newer composer"));
        let before = (current(&p), kept(&p));
        let effects = p.on_msg(Box::new(repeated));
        assert_eq!((current(&p), kept(&p)), before);
        assert!(
            effects
                .iter()
                .all(|effect| matches!(effect, Effect::Report(_) | Effect::Observe { .. }))
        );
        assert!(tasks(effects).is_empty());
        assert_eq!(recorded.requests().len(), 1);
    }
}

#[test]
fn old_account_success_records_the_dispatch_instance_without_current_account_fetch() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    p.replace_token("env:OTHER_SLACK_TOKEN".into());
    p.begin_reply(demo::TEAM, ROOT, Some("new account composer"));
    let before = (current(&p), kept(&p), p.data.clone());
    let effects = p.on_msg(Box::new(message));
    assert_eq!((current(&p), kept(&p), p.data.clone()), before);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Journal(action)
            if action.kind.as_str() == "slack.message.post.result"
            && action.subjects.iter().all(|subject| subject.key.instance == "T000ACME1")
    )));
    assert!(tasks(effects).is_empty());
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn an_old_failure_preserves_both_itself_and_a_newer_saved_reply() {
    let (mut p, recorded, _mode) = sample(true);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    p.begin_reply(demo::TEAM, ROOT, Some("newer saved reply"));
    let newer = preview(&mut p);
    p.answer_reply(&newer, "no").expect("save newer reply");
    let newer_id = *p.reply_context.kept.keys().next().unwrap();

    let effects = p.on_msg(Box::new(message));

    assert_eq!(
        p.reply_context.kept[&newer_id].original,
        "newer saved reply"
    );
    let mut texts = kept_texts(&p);
    texts.sort();
    let mut expected = vec![TEXT.to_string(), "newer saved reply".into()];
    expected.sort();
    assert_eq!(texts, expected);
    assert!(p.replying.is_none());
    assert!(tasks(effects).is_empty());
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn late_uncertain_results_preserve_every_version_without_touching_newer_composition() {
    let (mut p, _, _mode) = sample(true);
    let first_menu = preview(&mut p);
    let first = send(&mut p, &first_menu);
    p.begin_reply(demo::TEAM, ROOT, Some("second uncertain reply"));
    p.via = Via(Some(Recorded::new(vec![ok(json!({
        "ok": false, "error": "another_private_error_canary"
    }))])));
    let second_menu = preview(&mut p);
    let second = send(&mut p, &second_menu);
    p.begin_reply(demo::TEAM, ROOT, Some("third active reply"));
    let active = current(&p);

    p.on_msg(Box::new(second));
    p.on_msg(Box::new(first));

    assert_eq!(current(&p), active);
    let mut texts = kept_texts(&p);
    texts.sort();
    let mut expected = vec![TEXT.to_string(), "second uncertain reply".into()];
    expected.sort();
    assert_eq!(texts, expected);
    let current_menu = preview(&mut p);
    p.answer_reply(&current_menu, "no").expect("keep latest");
    let texts = kept_texts(&p);
    assert_eq!(texts.len(), 3);
    assert!(texts.iter().any(|text| text == "third active reply"));
}

#[test]
fn full_entry_capacity_refuses_save_send_and_replacement_without_losing_text() {
    for action in ["save", "send", "replace"] {
        let (mut p, recorded, _mode) = sample(false);
        discard_current(&mut p);
        for index in 0..MAX_KEPT_REPLIES {
            keep(
                &mut p,
                &format!("179120{:04}.000100", index),
                &format!("kept reply {index}"),
            );
        }
        assert_eq!(p.reply_context.usage().0, MAX_KEPT_REPLIES);
        p.begin_reply(demo::TEAM, ROOT, Some("active text at full capacity"));
        let before = (current(&p), kept(&p), p.reply_context.usage());
        let effects = if action == "replace" {
            p.begin_reply(demo::TEAM, OTHER_ROOT, Some("replacement seed"))
        } else {
            let menu = preview(&mut p);
            p.answer_reply(&menu, if action == "save" { "no" } else { "yes" })
                .expect("capacity refusal")
        };
        assert!(tasks(effects).is_empty());
        assert_eq!(
            (current(&p), kept(&p), p.reply_context.usage()),
            before,
            "{action}"
        );
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn capacity_counts_utf8_bytes_and_refuses_one_more_multibyte_character() {
    let (mut p, recorded, _mode) = sample(false);
    discard_current(&mut p);
    // A full real-sized boundary distinguishes bytes from Unicode characters.
    let full = "é".repeat(MAX_KEPT_BYTES / "é".len());
    assert_eq!(full.len(), MAX_KEPT_BYTES);
    keep(&mut p, ROOT, &full);
    assert_eq!(p.reply_context.usage(), (1, MAX_KEPT_BYTES));
    p.begin_reply(demo::TEAM, OTHER_ROOT, Some("é"));
    let before = (current(&p), kept(&p), p.reply_context.usage());

    let menu = preview(&mut p);
    let effects = p.answer_reply(&menu, "yes").expect("capacity refusal");

    assert!(tasks(effects).is_empty());
    assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
    assert!(recorded.requests().is_empty());
}

#[test]
fn active_send_reserves_recovery_capacity_until_its_success_is_applied() {
    let (mut p, recorded, _mode) = sample(false);
    discard_current(&mut p);
    for index in 0..MAX_KEPT_REPLIES - 1 {
        keep(&mut p, &format!("179120{:04}.000100", index), "kept");
    }
    p.begin_reply(demo::TEAM, ROOT, Some(TEXT));
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    assert_eq!(p.reply_context.usage().0, MAX_KEPT_REPLIES);
    p.begin_reply(demo::TEAM, OTHER_ROOT, Some("newer reply"));
    let before = (current(&p), kept(&p), p.reply_context.usage());
    let newer_menu = preview(&mut p);
    let blocked = p
        .answer_reply(&newer_menu, "yes")
        .expect("full reservation");
    assert!(tasks(blocked).is_empty());
    assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
    assert_eq!(recorded.requests().len(), 1);

    p.on_msg(Box::new(message));

    assert_eq!(p.reply_context.usage().0, MAX_KEPT_REPLIES - 1);
    assert_eq!(current(&p), before.0);
    let retry = preview(&mut p);
    assert_eq!(
        tasks(p.answer_reply(&retry, "yes").expect("space released")).len(),
        1
    );
    assert_eq!(p.reply_context.usage().0, MAX_KEPT_REPLIES);
}

#[test]
fn reply_history_never_contains_draft_or_error_text() {
    let (mut p, _, _mode) = sample(true);
    let mut records = String::new();
    let preview_effects = p.preview_reply();
    for effect in &preview_effects {
        if matches!(effect, Effect::Journal(_) | Effect::Observe { .. }) {
            records.push_str(&format!("{effect:?}"));
        }
    }
    let menu = menu_id(preview_effects, "reply-send:");
    let message = send(&mut p, &menu);
    for effect in p.on_msg(Box::new(message)) {
        if matches!(effect, Effect::Journal(_) | Effect::Observe { .. }) {
            records.push_str(&format!("{effect:?}"));
        }
    }
    assert!(!records.contains("private_reply_words_canary"));
    assert!(!records.contains("private_reply_error_canary"));
    assert!(!records.contains("second line"));
}

#[test]
fn uncertain_recovery_can_be_copied_or_edited_without_automatically_resending() {
    let (mut p, recorded, _mode) = sample(true);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    p.on_msg(Box::new(message));
    let id = *p.reply_context.kept.keys().next().unwrap();
    assert_eq!(p.reply_context.kept[&id].status, ReplyStatus::Uncertain);
    let before = kept(&p);
    let choice = entry(&mut p, id);
    let copied = p.answer_reply(&choice, "copy").expect("copy retained text");
    assert!(
        copied
            .iter()
            .any(|effect| matches!(effect, Effect::Copy(text) if text == TEXT))
    );
    assert_eq!(kept(&p), before);
    assert!(tasks(copied).is_empty());

    let choice = entry(&mut p, id);
    let opened = p.answer_reply(&choice, "edit").expect("recover text");
    assert!(tasks(opened).is_empty());
    assert_eq!(p.replying.as_ref().unwrap().composer.text(), TEXT);
    assert_eq!(p.replying.as_ref().unwrap().status, ReplyStatus::Uncertain);
    assert!(p.reply_context.kept.is_empty());
    let preview = p.preview_reply();
    assert!(menu_contains(&preview, "an earlier post may already exist"));
    assert!(tasks(preview).is_empty());
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn earlier_account_recovery_can_be_copied_but_not_silently_retargeted() {
    let (mut p, recorded, _mode) = sample(true);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    p.on_msg(Box::new(message));
    let id = *p.reply_context.kept.keys().next().unwrap();
    p.replace_token("env:OTHER_SLACK_TOKEN".into());
    p.begin_reply(demo::TEAM, OTHER_ROOT, Some("new context composer"));
    let before = (current(&p), kept(&p));
    let choice = entry(&mut p, id);

    let refused = p
        .answer_reply(&choice, "edit")
        .expect("old-context recovery refusal");

    assert!(tasks(refused).is_empty());
    assert_eq!((current(&p), kept(&p)), before);
    let choice = entry(&mut p, id);
    let copied = p
        .answer_reply(&choice, "copy")
        .expect("copy old-context text");
    assert!(
        copied
            .iter()
            .any(|effect| matches!(effect, Effect::Copy(text) if text == TEXT))
    );
    assert_eq!((current(&p), kept(&p)), before);
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn kept_reply_removal_is_confirmed_single_use_and_scoped_to_one_record() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("keep first");
    keep(&mut p, ROOT, "second retained reply");
    let ids: Vec<_> = p.reply_context.kept.keys().copied().collect();
    let choice = entry(&mut p, ids[0]);
    let question = menu_id(
        p.answer_reply(&choice, "drop")
            .expect("ask to remove one copy"),
        "reply-kept-drop:",
    );
    assert_eq!(p.reply_context.kept.len(), 2);
    p.answer_reply(&question, "yes")
        .expect("remove confirmed copy");
    assert!(!p.reply_context.kept.contains_key(&ids[0]));
    assert_eq!(
        p.reply_context.kept[&ids[1]].original,
        "second retained reply"
    );
    let before = kept(&p);
    assert_no_send(p.answer_reply(&question, "yes"), &recorded);
    assert_eq!(kept(&p), before);
}

#[test]
fn recovery_swaps_at_entry_capacity_without_discarding_the_open_composer() {
    let (mut p, recorded, _mode) = sample(false);
    discard_current(&mut p);
    for index in 0..MAX_KEPT_REPLIES {
        keep(
            &mut p,
            &format!("179120{:04}.000100", index),
            &format!("kept {index}"),
        );
    }
    let id = *p.reply_context.kept.keys().next().unwrap();
    let chosen = p.reply_context.kept[&id].original.clone();
    p.begin_reply(demo::TEAM, ROOT, Some("open work must survive"));
    let choice = entry(&mut p, id);

    let effects = p
        .answer_reply(&choice, "edit")
        .expect("swap retained reply");

    assert!(tasks(effects).is_empty());
    assert_eq!(p.replying.as_ref().unwrap().composer.text(), chosen);
    assert!(!p.reply_context.kept.contains_key(&id));
    assert_eq!(p.reply_context.usage().0, MAX_KEPT_REPLIES);
    assert!(
        kept_texts(&p)
            .iter()
            .any(|text| text == "open work must survive")
    );
    assert!(recorded.requests().is_empty());
}

#[test]
fn stale_recovery_keep_cannot_close_a_more_recently_edited_composer() {
    let (mut p, recorded, _mode) = sample(false);
    let list = menu_id(p.kept_replies(), "reply-kept:");
    p.paste("newer composition");
    let before = (current(&p), kept(&p), p.reply_context.usage());

    let effects = p.answer_reply(&list, "keep").expect("stale keep refusal");

    assert!(tasks(effects).is_empty());
    assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
    assert!(recorded.requests().is_empty());
}

#[test]
fn stale_recovery_new_or_entry_edit_cannot_replace_a_newer_composer() {
    for action in ["new", "edit"] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        p.answer_reply(&menu, "no").expect("keep original reply");
        let id = *p.reply_context.kept.keys().next().unwrap();
        p.begin_reply(demo::TEAM, OTHER_ROOT, Some("active work"));
        let choice = if action == "new" {
            menu_id(p.start_reply(demo::TEAM, ROOT), "reply-kept:")
        } else {
            entry(&mut p, id)
        };
        p.paste("newer changes");
        let before = (current(&p), kept(&p), p.reply_context.usage());

        let effects = p
            .answer_reply(&choice, action)
            .expect("stale recovery refusal");

        assert!(tasks(effects).is_empty());
        assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn host_ctrl_r_reaches_kept_replies_from_screen_or_offline_settings_while_composing() {
    for (configured, composing) in [(true, false), (true, true), (false, false), (false, true)] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        p.answer_reply(&menu, "no").expect("keep reply");
        if composing {
            p.begin_reply(demo::TEAM, OTHER_ROOT, Some("active offline draft"));
        }
        if !configured {
            p.cfg.token.clear();
            p.via = Via(None);
        }
        let mut host = crate::tests::hosted();
        host.plugins[0] = Box::new(p);
        host.view = if configured {
            View::Screen(0)
        } else {
            View::Settings
        };
        if !configured {
            host.section = host
                .sections
                .iter()
                .position(|(owner, section)| {
                    *owner == Owner::Plugin(0) && section.id == "connection"
                })
                .expect("Slack connection settings");
        }
        host.focus = Pane::Middle;
        let effects = host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        )));

        let id = host
            .menu
            .as_ref()
            .expect("offline recovery menu")
            .menu
            .id
            .clone();
        assert!(id.starts_with("reply-kept:"));
        assert_eq!(host.plugins[0].capturing(), composing);
        let _ = radar_core::shell::to_text(&host, 100, 30);
        let _ = radar_core::shell::to_text(&host, 80, 24);
        assert_eq!(host.menu.as_ref().unwrap().menu.id, id);
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, HostEffect::Spawn { .. }))
        );
        assert!(recorded.requests().is_empty());
    }
}

#[test]
fn recovery_edit_and_new_refuse_to_steal_another_editors_paste_target() {
    fn editor_text(p: &SlackPlugin, kind: &str) -> String {
        match kind {
            "mapping" => p.mapping.as_ref().unwrap().input.value(),
            "filter" => p.filter_input.as_ref().unwrap().value(),
            "settings" => p.edit.as_ref().unwrap().1.value(),
            "ask" => p.draft.as_ref().unwrap().edit.as_ref().unwrap().value(),
            _ => unreachable!(),
        }
    }

    for kind in ["mapping", "filter", "settings", "ask"] {
        for action in ["edit", "new"] {
            let (mut p, recorded, _mode) = sample(false);
            let menu = preview(&mut p);
            p.answer_reply(&menu, "no").expect("keep reply");
            let id = *p.reply_context.kept.keys().next().unwrap();
            match kind {
                "mapping" => {
                    p.mapping = Some(crate::Mapping {
                        name: "SyntheticAlert".into(),
                        input: TextInput::new("original"),
                    });
                }
                "filter" => p.filter_input = Some(TextInput::new("original")),
                "settings" => p.edit = Some((crate::Target::Alias(0), TextInput::new("original"))),
                "ask" => {
                    p.draft = Some(crate::ask::Draft {
                        edit: Some(TextInput::new("original")),
                        ..Default::default()
                    });
                }
                _ => unreachable!(),
            }
            let before = kept(&p);
            let Outcome::Handled(global) = p.key(
                "feed",
                Pane::Middle,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            ) else {
                panic!("Ctrl-R must open recovery while another editor is active")
            };
            let list = menu_id(global, "reply-kept:");
            let choice = if action == "edit" {
                menu_id(
                    p.answer_reply(&list, &format!("entry:{id}"))
                        .expect("kept entry"),
                    "reply-kept-entry:",
                )
            } else {
                menu_id(p.start_reply(demo::TEAM, ROOT), "reply-kept:")
            };

            let effects = p
                .answer_reply(&choice, action)
                .expect("other editor blocks recovery");

            assert!(tasks(effects).is_empty());
            assert!(p.replying.is_none(), "{kind}/{action}");
            assert_eq!(editor_text(&p, kind), "original", "{kind}/{action}");
            assert_eq!(kept(&p), before);
            p.paste(" more");
            assert_eq!(editor_text(&p, kind), "original more", "{kind}/{action}");
            assert!(p.replying.is_none());
            assert!(recorded.requests().is_empty());
        }
    }
}

#[test]
fn host_recovery_refusal_returns_paste_to_the_original_filter_editor() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("keep reply");
    p.filter = "original-filter".into();
    p.filter_input = Some(TextInput::new("original-filter"));
    let mut host = crate::tests::hosted();
    host.plugins[0] = Box::new(p);
    host.view = View::Screen(0);
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::CONTROL,
    )));
    let open = host.menu.as_mut().expect("recovery list");
    open.row = open
        .menu
        .items
        .iter()
        .position(|item| item.value.starts_with("entry:"))
        .unwrap();
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .id
            .starts_with("reply-kept-entry:")
    );
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::NONE,
    )));
    assert!(host.menu.is_none());
    assert!(host.plugins[0].capturing());
    host.update(Event::Paste(" suffix".into()));

    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: this test installed a SlackPlugin in slot zero above, and has
    // never replaced it. Inspect the actual input rather than a status line
    // that can temporarily be covered by the refusal notification.
    let p = unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) };
    assert_eq!(
        p.filter_input.as_ref().unwrap().value(),
        "original-filter suffix"
    );
    assert_eq!(p.filter, "original-filter suffix");
    assert!(p.replying.is_none());
    assert!(recorded.requests().is_empty());
}

#[test]
fn uncertain_status_survives_declining_and_reopening_recovered_text() {
    let (mut p, recorded, _mode) = sample(true);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    p.on_msg(Box::new(message));
    for _ in 0..2 {
        let id = *p.reply_context.kept.keys().next().unwrap();
        assert_eq!(p.reply_context.kept[&id].status, ReplyStatus::Uncertain);
        let choice = entry(&mut p, id);
        p.answer_reply(&choice, "edit")
            .expect("recover uncertain text");
        assert_eq!(p.replying.as_ref().unwrap().status, ReplyStatus::Uncertain);
        let effects = p.preview_reply();
        assert!(menu_contains(&effects, "an earlier post may already exist"));
        let menu = menu_id(effects, "reply-send:");
        p.answer_reply(&menu, "no")
            .expect("retain uncertain text again");
        assert!(p.replying.is_none());
        assert_eq!(kept_texts(&p), [TEXT]);
    }
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn an_empty_recovery_attempt_still_invalidates_an_earlier_send_menu() {
    let (mut p, recorded, _mode) = sample(false);
    let old = preview(&mut p);
    // Temporarily remove and restore the exact composer to isolate the pending
    // intent rule from the separate incarnation and revision protections.
    let composer = p.replying.take();
    let empty = p.kept_replies();
    assert!(!empty.iter().any(|effect| matches!(effect, Effect::Menu(_))));
    p.replying = composer;
    let before = current(&p);

    assert_no_send(p.answer_reply(&old, "yes"), &recorded);

    assert_eq!(current(&p), before);
}

#[test]
fn a_suspended_send_is_reachable_for_copy_without_releasing_its_reservation() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    let mut started = tasks(p.answer_reply(&menu, "yes").expect("accepted send"));
    assert_eq!(started.len(), 1);
    let pending = started.remove(0);
    assert!(
        recorded.requests().is_empty(),
        "the send task remains suspended"
    );
    let before = (current(&p), kept(&p), p.reply_context.usage());
    assert_eq!(before.2, (1, TEXT.len()));
    let (list, value) = pending_choice(p.start_reply(demo::TEAM, ROOT));
    let detail = p.answer_reply(&list, &value).expect("pending detail");
    assert!(menu_contains(
        &detail,
        "send pending; outcome unknown; do not retry"
    ));
    let choices: Vec<_> = detail
        .iter()
        .flat_map(|effect| match effect {
            Effect::Menu(menu) => menu
                .items
                .iter()
                .filter(|item| !item.value.is_empty())
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .collect();
    assert_eq!(choices, ["copy"]);
    let detail = menu_id(detail, "reply-pending-entry:");

    let copied = p
        .answer_reply(&detail, "copy")
        .expect("copy pending original");

    assert!(
        copied
            .iter()
            .any(|effect| matches!(effect, Effect::Copy(text) if text == TEXT))
    );
    assert!(tasks(copied).is_empty());
    assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
    assert!(recorded.requests().is_empty());
    let result = run(pending);
    p.on_msg(Box::new(result));
    assert_eq!(recorded.requests().len(), 1);
    assert_eq!(p.reply_context.usage(), (0, 0));
}

#[test]
fn pending_reply_detail_cannot_edit_drop_or_confirm_the_inflight_operation() {
    let (mut p, recorded, _mode) = sample(true);
    let menu = preview(&mut p);
    let mut started = tasks(p.answer_reply(&menu, "yes").expect("accepted send"));
    let pending = started.remove(0);
    let before = (current(&p), kept(&p), p.reply_context.usage());
    for action in ["edit", "drop", "yes"] {
        let (list, value) = pending_choice(p.kept_replies());
        let detail = menu_id(
            p.answer_reply(&list, &value).expect("pending detail"),
            "reply-pending-entry:",
        );

        let effects = p
            .answer_reply(&detail, action)
            .expect("unsupported pending action");

        assert!(tasks(effects).is_empty());
        assert_eq!(
            (current(&p), kept(&p), p.reply_context.usage()),
            before,
            "{action}"
        );
        assert!(recorded.requests().is_empty());
    }
    let result = run(pending);
    p.on_msg(Box::new(result));
    assert_eq!(kept_texts(&p), [TEXT]);
    assert_eq!(p.reply_context.usage(), before.2);
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn completion_makes_open_pending_list_and_detail_choices_harmless() {
    for failed in [false, true] {
        for detail_open in [false, true] {
            let (mut p, recorded, _mode) = sample(failed);
            let menu = preview(&mut p);
            let mut started = tasks(p.answer_reply(&menu, "yes").expect("accepted send"));
            let pending = started.remove(0);
            let (list, active_value) = pending_choice(p.kept_replies());
            let (choice, value) = if detail_open {
                (
                    menu_id(
                        p.answer_reply(&list, &active_value)
                            .expect("pending detail"),
                        "reply-pending-entry:",
                    ),
                    "copy".to_string(),
                )
            } else {
                (list, active_value.clone())
            };
            let result = run(pending);
            p.on_msg(Box::new(result));
            let before = (current(&p), kept(&p), p.reply_context.usage());

            let effects = p
                .answer_reply(&choice, &value)
                .expect("completed pending choice");

            assert!(
                !effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::Copy(_)))
            );
            assert!(effects.iter().any(|effect| matches!(effect,
                Effect::Notify(text) if text.contains("completed") && text.contains("open kept replies again"))));
            assert!(tasks(effects).is_empty());
            assert_eq!((current(&p), kept(&p), p.reply_context.usage()), before);
            assert_eq!(recorded.requests().len(), 1);
            if failed {
                assert_eq!(kept_texts(&p), [TEXT]);
                let fresh = p.kept_replies();
                assert!(
                    fresh
                        .iter()
                        .any(|effect| matches!(effect, Effect::Menu(menu)
                    if menu.items.iter().any(|item| item.value.starts_with("entry:"))
                    && menu.items.iter().all(|item| item.value != active_value)))
                );
            } else {
                assert!(p.reply_context.kept.is_empty());
                assert_eq!(p.reply_context.usage(), (0, 0));
            }
        }
    }
}

#[test]
fn offline_settings_editor_can_view_copy_and_drop_kept_text_without_becoming_a_reply() {
    fn choose(host: &mut radar_core::Host, value: &str) -> Vec<HostEffect> {
        let open = host.menu.as_mut().expect("a menu is open");
        open.row = open
            .menu
            .items
            .iter()
            .position(|item| item.value == value)
            .expect("requested menu action");
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
    }
    fn inspect(host: &radar_core::Host) -> &SlackPlugin {
        assert_eq!(host.plugins[0].id(), "slack");
        // SAFETY: this test installs one SlackPlugin in slot zero and never
        // replaces it. The cast only inspects this known fixture's state.
        unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) }
    }
    fn open_entry(host: &mut radar_core::Host, value: &str) {
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        )));
        assert!(
            host.menu
                .as_ref()
                .unwrap()
                .menu
                .id
                .starts_with("reply-kept:")
        );
        choose(host, value);
        assert!(
            host.menu
                .as_ref()
                .unwrap()
                .menu
                .id
                .starts_with("reply-kept-entry:")
        );
    }

    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("keep original text");
    let id = *p.reply_context.kept.keys().next().unwrap();
    let entry_value = format!("entry:{id}");
    // Save an actual target-scoped menu before entering settings. Delivering
    // its later "new" answer must not replace the settings input.
    let targeted = p
        .start_reply(demo::TEAM, ROOT)
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Menu(menu) => Some(menu),
            _ => None,
        })
        .expect("target-scoped recovery menu");
    p.cfg.token.clear();
    p.via = Via(None);
    p.cfg.review.template = "{mrs} original-template".into();
    p.section_rows[2] = 2; // Review's template row.
    let mut host = crate::tests::hosted();
    host.plugins[0] = Box::new(p);
    host.view = View::Settings;
    host.focus = Pane::Middle;
    host.section = host
        .sections
        .iter()
        .position(|(owner, section)| *owner == Owner::Plugin(0) && section.id == "review")
        .expect("Slack review settings");
    assert!(
        host.section_view_in(0, "review", 80, 40)
            .expect("neutral review settings")
            .actions
            .iter()
            .any(|action| action.id == "slack.review.kept_replies")
    );
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(matches!(
        inspect(&host).edit.as_ref(),
        Some((crate::Target::Template, _))
    ));
    host.menu = Some(OpenMenu::open(Owner::Plugin(0), targeted));
    choose(&mut host, "new");
    assert!(inspect(&host).replying.is_none());
    assert_eq!(
        inspect(&host).edit.as_ref().unwrap().1.value(),
        "{mrs} original-template"
    );

    open_entry(&mut host, &entry_value);
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .any(|item| item.label.contains("private_reply_words_canary"))
    );
    let copied = choose(&mut host, "copy");
    assert!(
        copied
            .iter()
            .any(|effect| matches!(effect, HostEffect::Copy(text) if text == TEXT))
    );
    assert_eq!(inspect(&host).reply_context.kept[&id].original, TEXT);

    open_entry(&mut host, &entry_value);
    let read_only = host.menu.take().expect("settings recovery detail");
    assert!(read_only.menu.items.iter().all(|item| item.value != "edit"));
    // The UI does not offer editing here; a fabricated callback must be
    // harmless as well, using the exact otherwise-current menu identity.
    let refused = host.plugins[0].on_menu(&read_only.menu.id, "edit");
    assert!(tasks(refused).is_empty());
    assert!(inspect(&host).replying.is_none());
    assert_eq!(
        inspect(&host).edit.as_ref().unwrap().1.value(),
        "{mrs} original-template"
    );

    open_entry(&mut host, &entry_value);
    choose(&mut host, "drop");
    assert!(
        host.menu
            .as_ref()
            .unwrap()
            .menu
            .id
            .starts_with("reply-kept-drop:")
    );
    assert!(inspect(&host).reply_context.kept.contains_key(&id));
    choose(&mut host, "yes");
    assert!(inspect(&host).reply_context.kept.is_empty());
    host.update(Event::Paste(" suffix".into()));
    let p = inspect(&host);
    assert!(p.replying.is_none());
    assert_eq!(p.edit.as_ref().unwrap().0, crate::Target::Template);
    assert_eq!(
        p.edit.as_ref().unwrap().1.value(),
        "{mrs} original-template suffix"
    );
    assert_eq!(p.cfg.review.template, "{mrs} original-template");
    assert!(recorded.requests().is_empty());
}

#[test]
fn configured_settings_recovery_without_an_editor_still_cannot_open_a_hidden_reply() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    p.answer_reply(&menu, "no").expect("retain reply");
    let id = *p.reply_context.kept.keys().next().unwrap();
    let before = kept(&p);
    assert!(!p.cfg.token.is_empty());
    assert!(!p.capturing());
    let mut host = crate::tests::hosted();
    host.plugins[0] = Box::new(p);
    host.view = View::Settings;
    host.focus = Pane::Middle;
    host.section = host
        .sections
        .iter()
        .position(|(owner, section)| *owner == Owner::Plugin(0) && section.id == "review")
        .expect("Slack review settings");
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::CONTROL,
    )));
    let list = host.menu.as_mut().expect("settings recovery list");
    assert!(list.menu.items.iter().all(|item| item.value != "new"));
    list.row = list
        .menu
        .items
        .iter()
        .position(|item| item.value == format!("entry:{id}"))
        .expect("retained record");
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let detail = host.menu.take().expect("settings recovery detail");
    assert!(detail.menu.items.iter().all(|item| item.value != "edit"));

    let refused = host.plugins[0].on_menu(&detail.menu.id, "edit");

    assert!(tasks(refused).is_empty());
    assert!(!host.plugins[0].capturing());
    assert_eq!(host.plugins[0].id(), "slack");
    // SAFETY: slot zero is the SlackPlugin installed in this fixture above;
    // no subsequent operation replaced it.
    let p = unsafe { &*(host.plugins[0].as_ref() as *const dyn Plugin as *const SlackPlugin) };
    assert!(p.replying.is_none());
    assert_eq!(kept(p), before);
    assert!(recorded.requests().is_empty());
}

#[test]
fn same_team_other_user_cannot_repreview_old_reply_and_text_stays_copyable() {
    let (mut p, recorded, _mode) = sample(false);
    let mut other = p.data.me.clone().unwrap();
    other.user_id = "U000OTHER".into();
    p.install_me(other);
    let effects = p.preview_reply();
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Journal(_)))
    );
    assert!(effects.iter().all(
        |effect| !matches!(effect, Effect::Observe { action, .. } if !action.subjects.is_empty())
    ));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Menu(_) | Effect::Spawn(_)))
    );
    assert_eq!(p.replying.as_ref().unwrap().composer.text(), TEXT);
    let menu = menu_id(p.kept_replies_in_settings(), "reply-kept:");
    let effects = p.answer_reply(&menu, "copy-current").unwrap();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Copy(text) if text == TEXT))
    );
    assert!(recorded.requests().is_empty());
}

#[test]
fn unverified_reply_cannot_preview_even_with_matching_cached_identity() {
    let (mut p, recorded, _mode) = sample(false);
    p.session_owner = None;
    let effects = p.preview_reply();
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Menu(_) | Effect::Spawn(_)))
    );
    assert_eq!(p.replying.as_ref().unwrap().composer.text(), TEXT);
    assert!(recorded.requests().is_empty());
}
