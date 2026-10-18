//! Delayed review-ask confirmations and replies must never consume newer work.
//! Every send uses the recorded transport; mode changes are thread-local and
//! restored even if a test fails.

use std::sync::{Arc, Mutex};

use radar_core::Plugin;
use radar_core::journal::Outcome as JournalOutcome;
use radar_core::plugin::{Effect, Msg, Pane, Task};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::writes::{self, Mode};
use serde_json::{Value, json};

use crate::ask::{Draft, Item};
use crate::client::{Recorded, Reply};
use crate::{SlackMsg, SlackPlugin, Via, Write, demo};

const POSTED: &str = "1791271000.000900";
const OLD_TEAM: &str = "T000ACME1";

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

fn item() -> Item {
    Item {
        id: "api!263".into(),
        url: demo::MR_263.into(),
        label: "api !263".into(),
        words: "private_review_words_canary".into(),
        tickets: vec![],
    }
}

fn second_item() -> Item {
    let mut other = item();
    other.id = "api!264".into();
    other.url = "https://git.example/acme/api/-/merge_requests/264".into();
    other.label = "api !264".into();
    other
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
    p.cfg.review.template = "{group} {@review-squad} {mrs}".into();
    let recorded = Recorded::new(vec![ok(if failed {
        json!({"ok": false, "error": "private_post_error_canary"})
    } else {
        json!({"ok": true, "channel": demo::TEAM, "ts": POSTED})
    })]);
    p.via = Via(Some(recorded.clone()));
    p.begin_ask_draft(Draft::new(&p.cfg, item()));
    p.pages = vec![crate::page::Page::Draft];
    (p, recorded, mode)
}

fn preview(p: &mut SlackPlugin) -> String {
    menu_id(p.preview_ask(), "ask-send:")
}

fn menu_id(effects: Vec<Effect>, prefix: &str) -> String {
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Menu(menu) => {
                assert!(menu.id.starts_with(prefix));
                Some(menu.id)
            }
            _ => None,
        })
        .expect("review confirmation")
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
    let mut started = tasks(p.answer_ask(menu, "yes").expect("ask answer"));
    assert_eq!(started.len(), 1, "one send for one confirmation");
    assert!(p.draft.as_ref().unwrap().sending.is_some());
    run(started.remove(0))
}

fn duplicate(message: &SlackMsg) -> SlackMsg {
    let SlackMsg::Wrote {
        instance,
        what,
        result,
    } = message
    else {
        panic!("write result");
    };
    SlackMsg::Wrote {
        instance: instance.clone(),
        what: what.clone(),
        result: result.clone(),
    }
}

fn assert_no_send(effects: Option<Vec<Effect>>, recorded: &Recorded) {
    assert!(tasks(effects.unwrap_or_default()).is_empty());
    assert!(recorded.requests().is_empty());
}

fn key(p: &mut SlackPlugin, code: KeyCode) {
    let _ = p.key(
        "feed",
        Pane::Middle,
        KeyEvent::new(code, KeyModifiers::NONE),
    );
}

fn content(p: &SlackPlugin) -> String {
    let d = p.draft.as_ref().unwrap();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}",
        d.stamp, d.items, d.channel, d.edit, d.error
    )
}

fn posted_reports(effects: &[Effect], expected: JournalOutcome) {
    assert!(
        matches!(effects.first(), Some(Effect::Report(outcome)) if *outcome == expected),
        "{effects:?}"
    );
}

#[test]
fn untouched_recorded_ask_success_records_once_and_failure_retains_retryable_words() {
    for failed in [false, true] {
        let (mut p, recorded, _mode) = sample(failed);
        let initial = p.draft.as_ref().unwrap().items.clone();
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        let effects = p.on_msg(Box::new(message));
        posted_reports(
            &effects,
            if failed {
                JournalOutcome::Unknown
            } else {
                JournalOutcome::Succeeded
            },
        );
        let requests = recorded.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "chat.postMessage");
        assert!(requests[0].write);
        assert_eq!(requests[0].body.as_ref().unwrap()["channel"], demo::TEAM);
        assert!(
            requests[0].body.as_ref().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("private_review_words_canary")
        );
        if failed {
            let d = p.draft.as_ref().unwrap();
            assert_eq!(d.items, initial);
            assert!(
                d.error
                    .as_deref()
                    .is_some_and(|error| error.contains("private_post_error_canary"))
            );
            assert_eq!(d.sending, None);
            assert!(p.data.asked.is_empty());
            assert_eq!(p.pages, [crate::page::Page::Draft]);
        } else {
            assert!(p.draft.is_none());
            assert!(p.pages.is_empty());
            assert_eq!(p.data.asked.len(), 1);
            assert_eq!(p.data.asked[0].channel, demo::TEAM);
            assert_eq!(p.data.asked[0].ts, POSTED);
            assert_eq!(
                p.data.asked[0].mrs,
                [(initial[0].id.clone(), initial[0].url.clone())]
            );
        }
    }
}

#[test]
fn successful_or_failed_old_send_cannot_consume_an_equal_recreated_draft() {
    for failed in [false, true] {
        let (mut p, _, _mode) = sample(failed);
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        let old_stamp = p.draft.as_ref().unwrap().stamp;
        p.draft = None;
        p.begin_ask_draft(Draft::new(&p.cfg, item()));
        assert_ne!(p.draft.as_ref().unwrap().stamp, old_stamp);
        p.draft.as_mut().unwrap().error = Some("new draft error".into());
        let before = content(&p);
        let effects = p.on_msg(Box::new(message));
        assert_eq!(content(&p), before);
        assert_eq!(p.draft.as_ref().unwrap().sending, None);
        assert_eq!(p.pages, [crate::page::Page::Draft]);
        assert_eq!(p.data.asked.len(), usize::from(!failed));
        posted_reports(
            &effects,
            if failed {
                JournalOutcome::Unknown
            } else {
                JournalOutcome::Succeeded
            },
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Words,
    Add,
    Remove,
    Channel,
    RevertedWords,
    Editor,
    Typed,
    Pasted,
}

fn mutate(p: &mut SlackPlugin, mutation: Mutation) {
    match mutation {
        Mutation::Words => {
            let d = p.draft.as_mut().unwrap();
            d.items[0].words = "new words".into();
            d.changed();
        }
        Mutation::Add => {
            p.draft.as_mut().unwrap().add(second_item());
        }
        Mutation::Remove => key(p, KeyCode::Char('d')),
        Mutation::Channel => {
            let menu = menu_id(p.preview_ask_channel(), "ask-channel:");
            p.answer_ask(&menu, "try").expect("channel selection");
        }
        Mutation::RevertedWords => {
            let d = p.draft.as_mut().unwrap();
            let original = d.items[0].words.clone();
            d.items[0].words = "temporary words".into();
            d.changed();
            d.items[0].words = original;
            d.changed();
        }
        Mutation::Editor | Mutation::Typed | Mutation::Pasted => {
            key(p, KeyCode::Char('e'));
            match mutation {
                Mutation::Typed => key(p, KeyCode::Char('x')),
                Mutation::Pasted => {
                    p.paste("new pasted words");
                }
                _ => {}
            }
        }
    }
}

#[test]
fn edited_drafts_survive_old_results_while_the_matching_busy_marker_is_retired() {
    for mutation in [
        Mutation::Words,
        Mutation::Add,
        Mutation::Remove,
        Mutation::Channel,
        Mutation::RevertedWords,
        Mutation::Editor,
        Mutation::Typed,
        Mutation::Pasted,
    ] {
        for failed in [false, true] {
            let (mut p, _, _mode) = sample(failed);
            let menu = preview(&mut p);
            let message = send(&mut p, &menu);
            mutate(&mut p, mutation);
            p.draft.as_mut().unwrap().error = Some("newer local error".into());
            let before = content(&p);
            p.on_msg(Box::new(message));
            assert_eq!(content(&p), before, "{mutation:?}, failed={failed}");
            assert_eq!(
                p.draft.as_ref().unwrap().sending,
                None,
                "own completed send must stop looking busy"
            );
            assert_eq!(p.pages, [crate::page::Page::Draft]);
            assert_eq!(p.data.asked.len(), usize::from(!failed));
        }
    }
}

#[test]
fn navigation_only_does_not_invalidate_a_send_or_pop_an_unrelated_page() {
    for leave_page in [false, true] {
        let (mut p, _, _mode) = sample(false);
        p.draft.as_mut().unwrap().add(second_item());
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        let stamp = p.draft.as_ref().unwrap().stamp;
        key(&mut p, KeyCode::Down);
        assert_eq!(p.draft.as_ref().unwrap().stamp, stamp);
        if leave_page {
            key(&mut p, KeyCode::Esc);
            p.pages.push(crate::page::Page::Thread {
                channel: demo::TEAM.into(),
                ts: POSTED.into(),
                tab: 0,
                row: 0,
            });
        }
        let pages = p.pages.clone();
        p.on_msg(Box::new(message));
        assert!(p.draft.is_none());
        assert_eq!(p.data.asked.len(), 1);
        if leave_page {
            assert_eq!(p.pages, pages);
        } else {
            assert!(p.pages.is_empty());
        }
    }
}

#[test]
fn duplicate_confirmation_and_completion_cannot_send_record_or_notify_twice() {
    for failed in [false, true] {
        let (mut p, recorded, _mode) = sample(failed);
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        assert!(tasks(p.answer_ask(&menu, "yes").unwrap_or_default()).is_empty());
        let repeated = duplicate(&message);
        p.on_msg(Box::new(message));
        p.begin_ask_draft(Draft::new(&p.cfg, second_item()));
        p.pages = vec![crate::page::Page::Draft];
        let before = content(&p);
        let effects = p.on_msg(Box::new(repeated));
        assert_eq!(content(&p), before);
        assert_eq!(p.pages, [crate::page::Page::Draft]);
        assert_eq!(p.data.asked.len(), usize::from(!failed));
        assert_eq!(recorded.requests().len(), 1);
        assert!(
            effects.iter().all(|effect| !matches!(
                effect,
                Effect::Notify(_) | Effect::NotifyWith(_, _) | Effect::Notice(_) | Effect::PersistCache
            )),
            "{effects:?}"
        );
    }
}

#[test]
fn old_menu_id_cannot_consume_the_latest_preview_and_each_answer_is_single_use() {
    let (mut p, recorded, _mode) = sample(false);
    let old = preview(&mut p);
    let latest = preview(&mut p);
    assert_ne!(old, latest);
    assert_no_send(p.answer_ask(&old, "yes"), &recorded);
    assert_no_send(p.answer_ask(&old, "edit"), &recorded);
    assert_no_send(p.answer_ask(&old, "no"), &recorded);
    let message = send(&mut p, &latest);
    assert!(tasks(p.answer_ask(&latest, "yes").unwrap_or_default()).is_empty());
    p.on_msg(Box::new(message));
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn draft_changes_between_preview_and_yes_refuse_the_old_confirmation() {
    for mutation in [
        Mutation::Words,
        Mutation::Add,
        Mutation::Remove,
        Mutation::Channel,
        Mutation::RevertedWords,
        Mutation::Editor,
        Mutation::Typed,
        Mutation::Pasted,
    ] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        mutate(&mut p, mutation);
        let before = content(&p);
        assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
        assert_eq!(content(&p), before, "{mutation:?}");
        assert_eq!(p.draft.as_ref().unwrap().sending, None);
    }
}

#[test]
fn derived_payload_drift_before_yes_never_sends_the_new_unreviewed_content() {
    for drift in 0..8 {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        match drift {
            0 => p.cfg.review.template.push_str(" changed template"),
            1 => p.cfg.review.group = "S000NEXT1".into(),
            2 => p.data.groups[0].id = "S000NEXT2".into(),
            3 => p.cfg.channels[0].id = "C000NEXT1".into(),
            4 => p.cfg.channels[0].kind = crate::config::Kind::Test,
            5 => p.cfg.review.test_channel = "team".into(),
            6 => p.draft.as_mut().unwrap().items[0].id = "api!999".into(),
            7 => p.draft.as_mut().unwrap().items[0].url = "https://git.example/new/mr/263".into(),
            _ => unreachable!(),
        }
        assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
        assert!(p.data.asked.is_empty());
        assert!(p.draft.is_some());
    }
}

#[test]
fn account_changes_including_a_b_a_invalidate_preview_and_preserve_post_send_drafts() {
    for after_send in [false, true] {
        for revert in [false, true] {
            let (mut p, recorded, _mode) = sample(false);
            let menu = preview(&mut p);
            let message = after_send.then(|| send(&mut p, &menu));
            let token = p.cfg.token.clone();
            let me = p.data.me.clone().unwrap();
            p.replace_token("env:SLACK_NEXT_TOKEN".into());
            if revert {
                p.replace_token(token);
                p.install_me(me);
            }
            if let Some(message) = message {
                let before = content(&p);
                p.on_msg(Box::new(message));
                assert_eq!(content(&p), before);
                assert_eq!(p.draft.as_ref().unwrap().sending, None);
                assert_eq!(p.data.asked.len(), usize::from(revert));
            } else {
                assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
            }
        }
    }
}

#[test]
fn mode_and_client_refusals_consume_confirmation_without_leaving_a_busy_draft() {
    for client_refusal in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        let token = p.cfg.token.clone();
        if client_refusal {
            // Empty literal forces a local client-construction error without
            // reading any environment value or constructing a live client.
            p.cfg.token.clear();
            p.via = Via(None);
        } else {
            writes::set_mode(Mode::ReadOnly, None);
        }
        assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
        assert_eq!(p.draft.as_ref().unwrap().sending, None);
        p.cfg.token = token;
        p.via = Via(Some(recorded.clone()));
        writes::set_mode(Mode::Edit, None);
        assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
    }
}

#[test]
fn a_completed_older_operation_cannot_retire_a_newer_sending_marker() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    let old = p.draft.as_ref().unwrap().sending.unwrap();
    p.begin_ask_draft(Draft::new(&p.cfg, second_item()));
    recorded.replies.lock().unwrap().push_back(ok(
        json!({"ok": true, "channel": demo::TEAM, "ts": "1791271001.000900"}),
    ));
    let newer_menu = preview(&mut p);
    let newer_message = send(&mut p, &newer_menu);
    let newer = p.draft.as_ref().unwrap().sending.unwrap();
    assert_ne!(old, newer);
    let before = content(&p);
    p.on_msg(Box::new(message));
    assert_eq!(content(&p), before);
    assert_eq!(p.draft.as_ref().unwrap().sending, Some(newer));
    assert_eq!(p.data.asked.len(), 1);
    p.on_msg(Box::new(newer_message));
    assert!(p.draft.is_none());
    assert_eq!(p.data.asked.len(), 2);
    assert_eq!(recorded.requests().len(), 2);
}

#[test]
fn unknown_operation_results_report_the_provider_outcome_without_mutating_drafts() {
    for failed in [false, true] {
        let (mut p, _, _mode) = sample(failed);
        let menu = preview(&mut p);
        let mut message = send(&mut p, &menu);
        let SlackMsg::Wrote {
            what: Write::Ask { operation },
            ..
        } = &mut message
        else {
            panic!("ask operation");
        };
        operation.id += 1000;
        let before = content(&p);
        let sending = p.draft.as_ref().unwrap().sending;
        let effects = p.on_msg(Box::new(message));
        posted_reports(
            &effects,
            if failed {
                JournalOutcome::Unknown
            } else {
                JournalOutcome::Succeeded
            },
        );
        assert_eq!(content(&p), before);
        assert_eq!(p.draft.as_ref().unwrap().sending, sending);
        assert!(p.data.asked.is_empty());
    }
}

#[test]
fn a_claimed_low_level_operation_cannot_dispatch_a_second_post() {
    let (mut p, recorded, _mode) = sample(false);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    let SlackMsg::Wrote { what, .. } = &message else {
        panic!("write result");
    };
    assert!(tasks(p.write(what.clone())).is_empty());
    assert_eq!(recorded.requests().len(), 1);
    p.on_msg(Box::new(message));
    assert_eq!(p.data.asked.len(), 1);
}

#[test]
fn edit_or_decline_consumes_only_its_preview_and_requires_a_new_yes() {
    for answer in ["edit", "no"] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = preview(&mut p);
        assert_no_send(p.answer_ask(&menu, answer), &recorded);
        assert_no_send(p.answer_ask(&menu, "yes"), &recorded);
        let new_menu = preview(&mut p);
        assert_ne!(new_menu, menu);
        let message = send(&mut p, &new_menu);
        p.on_msg(Box::new(message));
        assert_eq!(recorded.requests().len(), 1);
    }
}

#[derive(Clone, Copy, Debug)]
enum Dialog {
    Send,
    Drop,
    Channel,
}

fn dialog(p: &mut SlackPlugin, kind: Dialog) -> String {
    match kind {
        Dialog::Send => preview(p),
        Dialog::Drop => menu_id(p.preview_ask_drop(), "ask-drop:"),
        Dialog::Channel => menu_id(p.preview_ask_channel(), "ask-channel:"),
    }
}

fn choice(kind: Dialog) -> &'static str {
    match kind {
        Dialog::Send | Dialog::Drop => "yes",
        Dialog::Channel => "try",
    }
}

#[test]
fn stale_drop_and_channel_dialogs_cannot_change_recreated_revised_or_new_account_drafts() {
    for kind in [Dialog::Drop, Dialog::Channel] {
        for drift in 0..4 {
            let (mut p, recorded, _mode) = sample(false);
            let menu = dialog(&mut p, kind);
            match drift {
                0 => p.begin_ask_draft(Draft::new(&p.cfg, item())),
                1 => mutate(&mut p, Mutation::RevertedWords),
                2 => mutate(&mut p, Mutation::Editor),
                3 => {
                    let token = p.cfg.token.clone();
                    let me = p.data.me.clone().unwrap();
                    p.replace_token("env:SLACK_NEXT_TOKEN".into());
                    p.replace_token(token);
                    p.install_me(me);
                }
                _ => unreachable!(),
            }
            let before = content(&p);
            let pages = p.pages.clone();
            assert_no_send(p.answer_ask(&menu, choice(kind)), &recorded);
            assert_eq!(content(&p), before, "{kind:?}, drift={drift}");
            assert_eq!(p.pages, pages);
        }
    }
}

#[test]
fn older_dialogs_of_any_kind_cannot_consume_the_latest_send_drop_or_channel_intent() {
    for old_kind in [Dialog::Send, Dialog::Drop, Dialog::Channel] {
        for new_kind in [Dialog::Send, Dialog::Drop, Dialog::Channel] {
            let (mut p, recorded, _mode) = sample(false);
            let old = dialog(&mut p, old_kind);
            let latest = dialog(&mut p, new_kind);
            assert_ne!(old, latest);
            let before = content(&p);
            assert_no_send(p.answer_ask(&old, choice(old_kind)), &recorded);
            assert_eq!(content(&p), before);
            match new_kind {
                Dialog::Send => {
                    let message = send(&mut p, &latest);
                    p.on_msg(Box::new(message));
                    assert_eq!(recorded.requests().len(), 1);
                }
                Dialog::Drop => {
                    assert_no_send(p.answer_ask(&latest, "yes"), &recorded);
                    assert!(p.draft.is_none());
                    assert!(p.pages.is_empty());
                }
                Dialog::Channel => {
                    assert_no_send(p.answer_ask(&latest, "try"), &recorded);
                    assert_eq!(p.draft.as_ref().unwrap().channel, "try");
                    assert_ne!(content(&p), before);
                }
            }
        }
    }
}

#[test]
fn accepted_drop_and_channel_choices_are_single_use_and_cannot_touch_later_work() {
    for kind in [Dialog::Drop, Dialog::Channel] {
        let (mut p, recorded, _mode) = sample(false);
        let menu = dialog(&mut p, kind);
        assert_no_send(p.answer_ask(&menu, choice(kind)), &recorded);
        if matches!(kind, Dialog::Drop) {
            assert!(p.draft.is_none());
            p.begin_ask_draft(Draft::new(&p.cfg, item()));
            p.pages = vec![crate::page::Page::Draft];
        }
        let before = content(&p);
        let pages = p.pages.clone();
        // A duplicate channel choice even tries to move the selected channel back.
        let duplicate_choice = if matches!(kind, Dialog::Channel) {
            "team"
        } else {
            "yes"
        };
        assert_no_send(p.answer_ask(&menu, duplicate_choice), &recorded);
        assert_eq!(content(&p), before);
        assert_eq!(p.pages, pages);
    }
}

#[test]
fn dropping_an_inflight_draft_keeps_the_remote_outcome_and_never_claims_nothing_was_sent() {
    for failed in [false, true] {
        let (mut p, recorded, _mode) = sample(failed);
        let send_menu = preview(&mut p);
        let message = send(&mut p, &send_menu);
        let preview = p.preview_ask_drop();
        let preview_text = format!("{preview:?}");
        assert!(
            !preview_text.contains("nothing sent") && !preview_text.contains("nothing was sent"),
            "{preview_text}"
        );
        let drop_menu = menu_id(preview, "ask-drop:");
        let dropped = p.answer_ask(&drop_menu, "yes").expect("drop choice");
        let drop_text = format!("{dropped:?}");
        assert!(
            !drop_text.contains("nothing sent") && !drop_text.contains("nothing was sent"),
            "{drop_text}"
        );
        assert!(tasks(dropped).is_empty());
        assert!(p.draft.is_none());
        assert!(p.pages.is_empty());
        let completed = p.on_msg(Box::new(message));
        posted_reports(
            &completed,
            if failed {
                JournalOutcome::Unknown
            } else {
                JournalOutcome::Succeeded
            },
        );
        assert_eq!(p.data.asked.len(), usize::from(!failed));
        assert!(p.draft.is_none());
        assert_eq!(recorded.requests().len(), 1);
    }
}

struct AskHost(Arc<Mutex<SlackPlugin>>);
struct Preview;

impl Plugin for AskHost {
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
        match message.downcast::<Preview>() {
            Ok(_) => p.preview_ask(),
            Err(message) => p.on_msg(message),
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.0
            .lock()
            .unwrap()
            .answer_ask(menu, value)
            .unwrap_or_default()
    }
}

fn host_spawned(effects: &[radar_core::host::HostEffect]) -> bool {
    use radar_core::host::HostEffect;
    effects.iter().any(|effect| match effect {
        HostEffect::Spawn { .. } => true,
        HostEffect::Tracked { effect, .. } => matches!(**effect, HostEffect::Spawn { .. }),
        _ => false,
    })
}

#[test]
fn escaping_the_host_send_dialog_makes_a_later_yes_inert_and_reopening_has_a_fresh_id() {
    use radar_core::Host;
    use radar_core::host::Event;

    let (p, recorded, _mode) = sample(false);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AskHost(shared.clone()))], demo::NOW).unwrap();
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Preview),
    });
    let old = host.menu.as_ref().unwrap().menu.id.clone();
    let escaped = host.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(host.menu.is_none());
    assert!(!host_spawned(&escaped));
    let before = content(&shared.lock().unwrap());
    let later_yes = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(!host_spawned(&later_yes));
    assert!(recorded.requests().is_empty());
    assert_eq!(content(&shared.lock().unwrap()), before);
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Preview),
    });
    let new = host.menu.as_ref().unwrap().menu.id.clone();
    assert_ne!(new, old);
    assert!(new.starts_with("ask-send:"));
}

#[test]
fn stamped_group_refresh_while_host_preview_is_open_refuses_unreviewed_mention_changes() {
    use radar_core::Host;
    use radar_core::host::Event;

    let (p, recorded, _mode) = sample(false);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AskHost(shared.clone()))], demo::NOW).unwrap();
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Preview),
    });
    let menu = host.menu.as_ref().unwrap().menu.id.clone();
    let (task, group_recorded) = {
        let mut p = shared.lock().unwrap();
        let mut groups = p.data.groups.clone();
        groups[0].id = "S000FRESH1".into();
        let group_recorded = Recorded::new(vec![ok(json!({"ok": true, "usergroups": groups}))]);
        p.via = Via(Some(group_recorded.clone()));
        let task = tasks(crate::settings::fetch_groups(&mut p, false))
            .pop()
            .unwrap();
        (task, group_recorded)
    };
    let refreshed = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(run(task)),
    });
    assert!(!host_spawned(&refreshed));
    assert_eq!(shared.lock().unwrap().data.groups[0].id, "S000FRESH1");
    assert_eq!(host.menu.as_ref().unwrap().menu.id, menu);
    let before = content(&shared.lock().unwrap());
    let confirmed = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(!host_spawned(&confirmed));
    assert!(recorded.requests().is_empty());
    let requests = group_recorded.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "usergroups.list");
    assert!(!requests[0].write);
    let p = shared.lock().unwrap();
    assert_eq!(content(&p), before);
    assert_eq!(p.draft.as_ref().unwrap().sending, None);
    assert!(p.data.asked.is_empty());
}

#[test]
fn a_blocked_new_preview_attempt_invalidates_the_previous_pending_confirmation() {
    let (mut p, recorded, _mode) = sample(false);
    let old = preview(&mut p);
    let template = p.cfg.review.template.clone();
    p.cfg.review.template = "invalid template without its required placeholder".into();
    let blocked = p.preview_ask();
    assert!(
        blocked
            .iter()
            .all(|effect| !matches!(effect, Effect::Menu(_)))
    );
    assert!(
        blocked
            .iter()
            .any(|effect| matches!(effect, Effect::Report(JournalOutcome::Blocked)))
    );
    assert!(tasks(blocked).is_empty());
    p.cfg.review.template = template;
    // Draft, account and derived payload are now exactly as in the old preview.
    // The failed later attempt must nevertheless have retired that old intent.
    assert_no_send(p.answer_ask(&old, "yes"), &recorded);
    assert!(p.draft.is_some());
    assert_eq!(p.draft.as_ref().unwrap().sending, None);
}

#[test]
fn host_confirmation_and_delayed_success_keep_original_scope_without_leaking_payloads() {
    use radar_core::Host;
    use radar_core::host::{Event, HostEffect, JournalRecord};

    let (p, recorded, _mode) = sample(false);
    let shared = Arc::new(Mutex::new(p));
    let mut host = Host::new(vec![Box::new(AskHost(shared.clone()))], demo::NOW).unwrap();
    host.enable_journal();
    let previewed = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Preview),
    });
    let original = previewed
        .iter()
        .find_map(|effect| match effect {
            HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                if descriptor.kind.as_str() == "slack.review_ask.post" =>
            {
                assert_eq!(descriptor.subjects[0].key.instance, OLD_TEAM);
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(host.menu.as_ref().unwrap().menu.id.starts_with("ask-send:"));
    let confirmed = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    let (runtime, semantic, task) = confirmed
        .into_iter()
        .find_map(|effect| match effect {
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
        .unwrap();
    assert_eq!(semantic, original);
    let message = run(task);
    {
        let mut p = shared.lock().unwrap();
        let mut me = p.data.me.clone().unwrap();
        p.replace_token("env:SLACK_NEXT_TOKEN".into());
        me.team_id = "T000NEXT1".into();
        p.install_me(me);
        let draft = Draft::new(&p.cfg, second_item());
        p.begin_ask_draft(draft);
    }
    let before = content(&shared.lock().unwrap());
    let completed = host.update(Event::Tracked {
        action: runtime,
        semantic: Some(semantic),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(message),
        }),
    });
    let records: Vec<_> = completed
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record),
            _ => None,
        })
        .collect();
    let terminal: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome { id, outcome, .. }
                if id == &original && outcome.is_terminal() =>
            {
                Some(*outcome)
            }
            _ => None,
        })
        .collect();
    assert_eq!(terminal, [JournalOutcome::Succeeded]);
    let posted = records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { descriptor, .. }
                if descriptor.kind.as_str() == "slack.message.post.result" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        posted
            .subjects
            .iter()
            .all(|subject| subject.key.instance == OLD_TEAM)
    );
    assert!(!format!("{records:?}").contains("private_"));
    let p = shared.lock().unwrap();
    assert_eq!(content(&p), before);
    assert!(p.data.asked.is_empty());
    assert_eq!(recorded.requests().len(), 1);
}

#[test]
fn old_ask_cannot_be_freshly_previewed_for_another_account_with_matching_ids() {
    for same_team in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let original = content(&p);
        let mut other = p.data.me.clone().unwrap();
        other.user_id = "U000OTHER".into();
        if !same_team {
            other.team_id = "T000OTHER".into();
        }
        p.install_me(other);
        assert!(p.ask_draft_blocked().is_some());
        let effects = p.preview_ask();
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Journal(_)))
        );
        assert!(effects.iter().all(|effect| !matches!(effect, Effect::Observe { action, .. } if !action.subjects.is_empty())));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::Menu(_) | Effect::Spawn(_)))
        );
        assert_eq!(content(&p), original);
        assert!(recorded.requests().is_empty());
        let copied = p.copy_ask_draft();
        assert!(copied.iter().any(|effect| matches!(effect, Effect::Copy(text) if text.contains("private_review_words_canary") && text.contains(demo::MR_263))));
    }
}

#[test]
fn late_success_is_parked_for_the_original_account_not_the_current_same_id_target() {
    for same_team in [false, true] {
        let (mut p, recorded, _mode) = sample(false);
        let original_me = p.data.me.clone().unwrap();
        let menu = preview(&mut p);
        let message = send(&mut p, &menu);
        let mut other = original_me.clone();
        other.user_id = "U000OTHER".into();
        if !same_team {
            other.team_id = "T000OTHER".into();
        }
        p.install_me(other);
        let original = content(&p);
        let effects = p.on_msg(Box::new(message));
        assert_eq!(content(&p), original);
        assert!(p.data.asked.is_empty());
        assert!(tasks(effects).is_empty());
        assert_eq!(recorded.requests().len(), 1);
        p.install_me(original_me);
        assert_eq!(p.data.asked.len(), 1);
        assert_eq!(p.data.asked[0].ts, POSTED);
        assert!(
            p.ask_draft_blocked().is_some(),
            "returning to A does not silently re-enable a possibly posted draft"
        );
    }
}

#[test]
fn unverified_ask_keeps_authored_words_without_adopting_the_next_verified_owner() {
    let (mut p, recorded, _mode) = sample(false);
    let me = p.data.me.clone().unwrap();
    p.replace_token("env:SLACK_UNVERIFIED_TOKEN".into());
    p.begin_ask_draft(Draft::new(&p.cfg, item()));
    p.install_me(me);
    assert!(p.ask_draft_blocked().is_some());
    assert!(
        !p.preview_ask()
            .iter()
            .any(|effect| matches!(effect, Effect::Menu(_)))
    );
    assert!(p.copy_ask_draft().iter().any(|effect| matches!(effect, Effect::Copy(text) if text.contains("private_review_words_canary"))));
    assert!(recorded.requests().is_empty());
}

#[test]
fn boundary_control_old_draft_cannot_get_a_new_account_preview() {
    let (mut p, _, _mode) = sample(false);
    let mut other = p.data.me.clone().unwrap();
    other.user_id = "UOTHER".into();
    p.install_me(other);
    assert!(!p.preview_ask().iter().any(|e| matches!(e, Effect::Menu(_))));
}

#[test]
fn boundary_control_late_ask_success_does_not_enter_new_account_data() {
    let (mut p, _, _mode) = sample(false);
    let menu = preview(&mut p);
    let message = send(&mut p, &menu);
    let mut other = p.data.me.clone().unwrap();
    other.user_id = "UOTHER".into();
    p.install_me(other);
    p.on_msg(Box::new(message));
    assert!(!p.data.asked.iter().any(|a| a.ts == POSTED));
}
