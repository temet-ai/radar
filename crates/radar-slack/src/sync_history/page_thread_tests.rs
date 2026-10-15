//! Page reads use the recorded transport; no test can reach Slack.

use std::sync::{Arc, Mutex};

use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionDescriptor, ActionId, Outcome, Reason, SubjectRole};
use radar_core::plugin::{Effect, Msg, Task};
use radar_core::writes::{self, Mode};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

use super::{page_thread, page_thread_action};
use crate::client::{ListEvidence, Message, MessagesRead, Recorded, Reply};
use crate::{SlackPlugin, Via, demo, page, thread_key};

const TEAM: &str = "T000ACME1";
const ROOT: &str = "1791200000.000100";
const OTHER: &str = "1791200001.000100";
const PREFIX: &str = "slack.page.thread";
const PRIVATE: &str = "PRIVATE_PAGE_CANARY";

fn ok(body: Value) -> anyhow::Result<Reply> {
    Ok(Reply {
        status: 200,
        body: body.to_string(),
        ..Default::default()
    })
}
fn rows(count: usize, start: usize) -> Vec<Value> {
    (start..start + count).map(|n| json!({
        "ts": format!("179120{:04}.000100", n), "text": PRIVATE,
        "user": PRIVATE, "permalink": "https://private.example/PRIVATE_PAGE_CANARY?token=secret"
    })).collect()
}
fn response(messages: Vec<Value>, next: &str) -> anyhow::Result<Reply> {
    ok(json!({"ok": true, "messages": messages, "response_metadata": {"next_cursor": next}}))
}
fn sample(replies: Vec<anyhow::Result<Reply>>) -> (SlackPlugin, Arc<Recorded>) {
    let mut p = demo::sample();
    let rec = Recorded::new(replies);
    p.via = Via(Some(rec.clone()));
    p.data.channels.get_mut(demo::TEAM).unwrap().messages =
        vec![demo::msg(ROOT, demo::BOB, "held parent")];
    p.data.channels.get_mut(demo::TEAM).unwrap().replies.clear();
    (p, rec)
}
fn take_task(effects: Vec<Effect>) -> (ActionDescriptor, Task) {
    let mut tasks = effects.into_iter().filter_map(|effect| match effect {
        Effect::SpawnAction { action, task } => Some((action, task)),
        Effect::Spawn(_) => panic!("page reads must carry a semantic action"),
        _ => None,
    });
    let got = tasks.next().expect("one page read");
    assert!(tasks.next().is_none());
    got
}
fn run(task: Task) -> Msg {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(task)
}
fn fetch(p: &mut SlackPlugin) -> Vec<Effect> {
    let (_, task) = take_task(page::fetch_thread(p, demo::TEAM, ROOT));
    p.on_msg(run(task))
}
fn history(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::Report(_) | Effect::Observe { .. } | Effect::Journal(_)
            )
        })
        .collect()
}
fn report(effects: &[Effect], expected: Outcome) {
    let reports: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Report(outcome) => Some(*outcome),
            _ => None,
        })
        .collect();
    assert_eq!(reports, [expected]);
    assert!(!format!("{:?}", history(effects)).contains(PRIVATE));
}
fn count(effects: &[Effect], suffix: &str, expected: u32, outcome: Outcome, truncated: bool) {
    let kind = format!("{PREFIX}.{suffix}");
    let counts: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
                Some((action, *outcome))
            }
            _ => None,
        })
        .collect();
    assert_eq!(counts.len(), 1, "{kind}: {effects:?}");
    assert_eq!(
        (
            counts[0].0.metadata.count,
            counts[0].1,
            counts[0].0.metadata.truncated
        ),
        (Some(expected), outcome, truncated)
    );
}
fn scope(action: &ActionDescriptor, team: &str, channel: &str, root: &str) {
    assert_eq!(action.subjects.len(), 1);
    let subject = &action.subjects[0];
    assert_eq!(subject.key.provider, "slack");
    assert_eq!(subject.key.instance, team);
    assert_eq!(subject.key.kind, "thread");
    assert_eq!(subject.key.id, format!("{channel}/{root}"));
    assert_eq!(subject.role, SubjectRole::Primary);
    assert!(subject.url.is_none());
}

#[test]
fn page_scope_reuses_validated_thread_identity_and_omits_unsafe_or_unknown_scope() {
    scope(
        &page_thread_action(TEAM, demo::TEAM, ROOT),
        TEAM,
        demo::TEAM,
        ROOT,
    );
    for (team, channel, root) in [
        ("", demo::TEAM, ROOT),
        ("T", demo::TEAM, ROOT),
        (PRIVATE, demo::TEAM, ROOT),
        (TEAM, PRIVATE, ROOT),
        (TEAM, demo::TEAM, PRIVATE),
        (TEAM, demo::TEAM, "12"),
        (TEAM, demo::TEAM, "12."),
        (TEAM, demo::TEAM, ".1"),
        (TEAM, demo::TEAM, "12.1?PRIVATE_PAGE_CANARY"),
    ] {
        let action = page_thread_action(team, channel, root);
        assert!(action.subjects.is_empty());
        assert!(action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
        assert!(!format!("{action:?}").contains(PRIVATE));
    }
    assert_ne!(
        page_thread_action(TEAM, demo::TEAM, ROOT).subjects,
        page_thread_action("T000NEXT1", demo::TEAM, ROOT).subjects
    );
}

#[test]
fn complete_empty_and_missing_parent_lists_are_successful_without_parent_existence_claims() {
    for messages in [vec![], vec![json!({"ts": OTHER, "text": PRIVATE})]] {
        let n = messages.len();
        let (mut p, rec) = sample(vec![response(messages, "")]);
        let effects = fetch(&mut p);
        report(&effects, Outcome::Succeeded);
        count(&effects, "messages", n as u32, Outcome::Succeeded, false);
        assert_eq!(history(&effects).len(), 2);
        let held = &p.threads[&thread_key(demo::TEAM, ROOT)];
        assert!(!held.loading && held.error.is_none());
        assert_eq!(held.value.as_ref().unwrap().len(), n);
        assert_eq!(p.data.channels[demo::TEAM].messages[0].text, "held parent");
        assert_eq!(rec.requests().len(), 1);
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
    }
}

#[test]
fn page_reads_preserve_500_entry_bound_and_200_request_limit_including_parent() {
    for (last_count, next, truncated) in [(100, "", false), (101, "", true), (100, "c4", true)] {
        let (mut p, rec) = sample(vec![
            response(rows(200, 0), "c2"),
            response(rows(200, 200), "c3"),
            response(rows(last_count, 400), next),
        ]);
        let effects = fetch(&mut p);
        let outcome = if truncated {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        report(&effects, outcome);
        count(&effects, "messages", 500, outcome, truncated);
        if truncated {
            count(&effects, "lists.truncated", 1, Outcome::Partial, true);
        }
        assert_eq!(
            p.threads[&thread_key(demo::TEAM, ROOT)]
                .value
                .as_ref()
                .unwrap()
                .len(),
            500
        );
        // The requested parent is the first of those 500 entries, not an extra row.
        assert_eq!(p.data.channels[demo::TEAM].replies[ROOT].len(), 499);
        let requests = rec.requests();
        assert_eq!(requests.len(), 3);
        for (i, request) in requests.iter().enumerate() {
            assert_eq!(request.method, "conversations.replies");
            let mut params = vec![
                ("channel".into(), demo::TEAM.into()),
                ("ts".into(), ROOT.into()),
                ("limit".into(), "200".into()),
            ];
            if i > 0 {
                params.push(("cursor".into(), format!("c{}", i + 1)));
            }
            assert_eq!(request.params, params);
        }
    }
}

#[test]
fn page_cursor_bound_keeps_actual_last_page_evidence_without_an_extra_request() {
    for unread in [false, true] {
        let replies = (0..20)
            .map(|i| {
                response(
                    rows(1, i),
                    &if i == 19 && !unread {
                        String::new()
                    } else {
                        format!("c{}", i + 2)
                    },
                )
            })
            .collect();
        let (mut p, rec) = sample(replies);
        let effects = fetch(&mut p);
        let outcome = if unread {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        report(&effects, outcome);
        count(&effects, "messages", 20, outcome, unread);
        assert_eq!(rec.requests().len(), 20);
    }
}

#[test]
fn incomplete_page_is_partial_not_an_invented_empty_remote_thread() {
    for body in [
        json!({"ok": true}),
        json!({"ok": true, "messages": null}),
        json!({"ok": true, "messages": [], "response_metadata": {"next_cursor": 7}, "has_more": true}),
    ] {
        let (mut p, rec) = sample(vec![ok(body)]);
        let effects = fetch(&mut p);
        report(&effects, Outcome::Partial);
        count(&effects, "messages", 0, Outcome::Partial, false);
        count(&effects, "pages.incomplete", 1, Outcome::Partial, false);
        assert_eq!(history(&effects).len(), 3);
        assert_eq!(rec.requests().len(), 1);
    }
}

#[test]
fn later_page_error_preserves_old_values_and_has_no_inferred_progress_counts() {
    let (mut p, rec) = sample(vec![
        response(rows(2, 0), "c2"),
        ok(json!({"ok": false, "error": PRIVATE})),
    ]);
    let held = vec![demo::msg(ROOT, demo::BOB, "previous value")];
    p.threads
        .entry(thread_key(demo::TEAM, ROOT))
        .or_default()
        .value = Some(held.clone());
    let before = p.data.clone();
    let effects = fetch(&mut p);
    report(&effects, Outcome::Failed);
    count(&effects, "errors", 1, Outcome::Failed, false);
    assert_eq!(history(&effects).len(), 2);
    assert_eq!(p.data, before);
    let fetched = &p.threads[&thread_key(demo::TEAM, ROOT)];
    assert!(!fetched.loading && fetched.error.is_some());
    assert_eq!(fetched.value.as_ref().unwrap(), &held);
    assert_eq!(rec.requests().len(), 2);
}

#[test]
fn client_refusal_precedes_loading_guard_and_suppression_does_not_finish_original_read() {
    let (mut p, rec) = sample(vec![response(rows(1, 0), "")]);
    let (_, task) = take_task(page::fetch_thread(&mut p, demo::TEAM, ROOT));
    let effects = page::fetch_thread(&mut p, demo::TEAM, ROOT);
    count(&effects, "suppressed", 1, Outcome::Blocked, false);
    assert!(
        matches!(&effects[..], [Effect::Observe { .. }, Effect::Notify(s)] if s == "already reading this thread")
    );
    assert!(p.threads[&thread_key(demo::TEAM, ROOT)].loading);
    assert!(rec.requests().is_empty());
    p.via = Via(None);
    p.cfg.token.clear();
    let effects = page::fetch_thread(&mut p, demo::TEAM, ROOT);
    count(&effects, "unavailable", 1, Outcome::Blocked, false);
    assert!(
        matches!(&effects[..], [Effect::Observe { action, outcome: Outcome::Blocked }] if action.metadata.reason == Some(Reason::Unavailable))
    );
    assert!(p.threads[&thread_key(demo::TEAM, ROOT)].loading);
    assert!(rec.requests().is_empty());
    let effects = p.on_msg(run(task));
    report(&effects, Outcome::Succeeded);
    assert!(!p.threads[&thread_key(demo::TEAM, ROOT)].loading);
    assert_eq!(rec.requests().len(), 1);
    // A fresh unavailable invocation also leaves the thread map untouched.
    let effects = page::fetch_thread(&mut p, demo::SVC, OTHER);
    count(&effects, "unavailable", 1, Outcome::Blocked, false);
    assert!(!p.threads.contains_key(&thread_key(demo::SVC, OTHER)));
}

#[test]
fn held_whole_thread_opens_without_request_and_fetched_open_thread_marks_returned_rows_read() {
    let (mut p, rec) = sample(vec![response(
        vec![
            json!({"ts": ROOT, "text": PRIVATE, "reply_count": 1}),
            json!({"ts": OTHER, "thread_ts": ROOT, "text": PRIVATE}),
        ],
        "",
    )]);
    let effects = page::open_thread(&mut p, demo::TEAM, ROOT);
    assert!(
        matches!(history(&effects)[..], [Effect::Observe { action, outcome: Outcome::Succeeded }]
        if action.kind.as_str() == "slack.page.open")
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert!(rec.requests().is_empty());
    let effects = fetch(&mut p);
    report(&effects, Outcome::Succeeded);
    count(&effects, "messages", 2, Outcome::Succeeded, false);
    assert_eq!(p.data.channels[demo::TEAM].messages[0].text, PRIVATE);
    assert_eq!(p.data.channels[demo::TEAM].replies[ROOT][0].ts, OTHER);
    assert!(!p.data.is_new(demo::TEAM, ROOT, OTHER));
    assert_eq!(rec.requests().len(), 1);
    assert!(!p.snapshot().unwrap().to_string().contains(PREFIX));
}

#[test]
fn unavailable_identity_blocks_page_reads_without_recorded_requests() {
    let (mut p, rec) = sample(vec![response(vec![], "")]);
    p.data.me = None;
    let effects = page::fetch_thread(&mut p, demo::TEAM, ROOT);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert!(rec.requests().is_empty());
    assert!(history(&effects).is_empty());
}

struct ModeGuard((Mode, Option<String>));
impl ModeGuard {
    fn edit() -> Self {
        let old = writes::mode();
        writes::set_mode(Mode::Edit, None);
        Self(old)
    }
}
impl Drop for ModeGuard {
    fn drop(&mut self) {
        writes::set_mode(self.0.0, self.0.1.clone());
    }
}

enum Command {
    Page(&'static str, &'static str),
    Write(bool),
}
struct PageHost(Arc<Mutex<SlackPlugin>>);
fn menu(effects: &[Effect]) -> String {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Menu(m) => Some(m.id.clone()),
            _ => None,
        })
        .expect("write menu")
}
impl Plugin for PageHost {
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
            Ok(command) => match *command {
                Command::Page(channel, root) => page::fetch_thread(&mut p, channel, root),
                Command::Write(reply) => {
                    // Exercise actual one-use intents and real recorded write callbacks.
                    let preview = if reply {
                        p.begin_reply(demo::TEAM, ROOT, Some(PRIVATE));
                        p.preview_reply()
                    } else {
                        let pick = menu(&page::react_menu(&mut p, demo::TEAM, ROOT));
                        p.answer_reaction(&pick, "eyes").unwrap()
                    };
                    let id = menu(&preview);
                    let mut effects: Vec<_> = preview
                        .into_iter()
                        .filter(|e| !matches!(e, Effect::Menu(_)))
                        .collect();
                    effects.extend(
                        if reply {
                            p.answer_reply(&id, "yes")
                        } else {
                            p.answer_reaction(&id, "yes")
                        }
                        .unwrap(),
                    );
                    effects
                }
            },
        }
    }
}
fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}
fn terminals(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|r| match r {
            JournalRecord::Outcome {
                id: target,
                outcome,
                ..
            } if target == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}
fn tracked(effects: Vec<HostEffect>) -> (ActionId, ActionId, Task) {
    let mut tasks = effects.into_iter().filter_map(|effect| match effect {
        HostEffect::Tracked {
            action,
            semantic: Some(semantic),
            effect,
        } => match *effect {
            HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
            _ => None,
        },
        HostEffect::Spawn { .. } => panic!("untracked task"),
        _ => None,
    });
    let task = tasks.next().expect("one tracked task");
    assert!(tasks.next().is_none());
    task
}
fn deliver(host: &mut Host, action: ActionId, semantic: ActionId, msg: Msg) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action,
        semantic: Some(semantic),
        event: Box::new(Event::Msg { plugin: 0, msg }),
    })
}
fn held(p: &SlackPlugin) -> String {
    format!("{:?}", (&p.data, &p.pages, &p.threads, &p.sync, &p.checks))
}
fn persist_and_check(records: &[JournalRecord]) {
    assert!(!format!("{records:?}").contains(PRIVATE));
    #[cfg(unix)]
    {
        use radar_core::journal::{Journal, Query};
        let dir = std::env::temp_dir().join(format!(
            "radar-slack-page-history-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in records {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, id, descriptor.clone(), parent.as_ref())
                    .unwrap(),
                JournalRecord::Outcome {
                    id,
                    outcome,
                    metadata,
                } => {
                    journal.record(id, *outcome, metadata.clone()).unwrap();
                }
                JournalRecord::Gap { .. } => panic!("unexpected gap"),
            }
        }
        let events = journal
            .query(&Query {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let exported = serde_json::to_string(&events).unwrap();
        for private in [PRIVATE, "private.example", "eyes", "xoxp-", "PRIVATE_ERROR"] {
            assert!(
                !exported.contains(private),
                "persisted private payload: {exported}"
            );
        }
        assert!(
            events
                .iter()
                .any(|event| event.kind.as_str().starts_with(PREFIX))
        );
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn actual_reply_and_reaction_success_stay_successful_after_every_child_read_outcome() {
    let _mode = ModeGuard::edit();
    for reply in [false, true] {
        for child in [
            "failed",
            "partial",
            "unavailable",
            "suppressed",
            "succeeded",
        ] {
            let (p, rec) = sample(vec![
                ok(json!({"ok": true, "channel": demo::TEAM, "ts": OTHER})),
                ok(match child {
                    "failed" => json!({"ok": false, "error": PRIVATE}),
                    "partial" => json!({"ok": true, "messages": null}),
                    _ => json!({"ok": true, "messages": []}),
                }),
            ]);
            let shared = Arc::new(Mutex::new(p));
            let mut host = Host::new(vec![Box::new(PageHost(shared.clone()))], demo::NOW).unwrap();
            host.enable_journal();
            let start = host.update(Event::Msg {
                plugin: 0,
                msg: Box::new(Command::Write(reply)),
            });
            let mut all = records(&start);
            let (write_task, write, task) = tracked(start);
            let write_kind = if reply {
                "slack.reply.post"
            } else {
                "slack.reaction.add"
            };
            assert!(all.iter().any(|r| matches!(r, JournalRecord::Begin { id, descriptor, .. } if id == &write && descriptor.kind.as_str() == write_kind)));
            let message = run(task);
            assert_eq!(rec.requests().len(), 1);
            {
                let mut p = shared.lock().unwrap();
                if child == "unavailable" {
                    // A local client-construction failure without a generation change.
                    p.via = Via(None);
                    p.cfg.token.clear();
                } else if child == "suppressed" {
                    p.threads
                        .entry(thread_key(demo::TEAM, ROOT))
                        .or_default()
                        .loading = true;
                }
            }
            let completed = deliver(&mut host, write_task.clone(), write.clone(), message);
            all.extend(records(&completed));
            assert_eq!(terminals(&all, &write), [Outcome::Succeeded]);
            assert_eq!(terminals(&all, &write_task), [Outcome::Succeeded]);
            if matches!(child, "unavailable" | "suppressed") {
                assert!(!completed.iter().any(|e| matches!(e, HostEffect::Tracked { effect, .. } if matches!(&**effect, HostEffect::Spawn { .. }))));
                let refusal = all
                    .iter()
                    .find_map(|r| match r {
                        JournalRecord::Begin { id, descriptor, .. }
                            if descriptor.kind.as_str() == format!("{PREFIX}.{child}") =>
                        {
                            Some(id)
                        }
                        _ => None,
                    })
                    .expect("independent refusal");
                assert_eq!(terminals(&all, refusal), [Outcome::Blocked]);
                assert_eq!(rec.requests().len(), 1);
            } else {
                let (read_task, read, task) = tracked(completed);
                assert_ne!(read, write);
                assert_ne!(read_task, write_task);
                assert!(all.iter().any(|r| matches!(r, JournalRecord::Begin { id, descriptor, .. } if id == &read && descriptor.kind.as_str() == PREFIX)));
                let response = run(task);
                let result = deliver(&mut host, read_task.clone(), read.clone(), response);
                let got = records(&result);
                let expected = match child {
                    "failed" => Outcome::Failed,
                    "partial" => Outcome::Partial,
                    _ => Outcome::Succeeded,
                };
                assert_eq!(terminals(&got, &read), [expected]);
                assert_eq!(terminals(&got, &read_task), [expected]);
                assert!(terminals(&got, &write).is_empty());
                all.extend(got);
                assert_eq!(rec.requests().len(), 2);
            }
            assert_eq!(
                terminals(&all, &write),
                [Outcome::Succeeded],
                "{write_kind} / {child}"
            );
            assert_eq!(terminals(&all, &write_task), [Outcome::Succeeded]);
            assert!(!all.iter().any(|record| matches!(record,
                JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "slack.page.open")));
            persist_and_check(&all);
        }
    }
}

#[test]
fn out_of_order_page_results_keep_dispatch_scope_and_stale_result_evidence_in_sqlite() {
    for stale in [false, true] {
        for first_outcome in [Outcome::Succeeded, Outcome::Partial, Outcome::Failed] {
            let first = match first_outcome {
                Outcome::Succeeded => {
                    json!({"ok": true, "messages": [{"ts": ROOT, "text": PRIVATE}]})
                }
                Outcome::Partial => {
                    json!({"ok": true, "messages": [{"ts": ROOT, "text": PRIVATE}], "has_more": true})
                }
                _ => json!({"ok": false, "error": PRIVATE}),
            };
            let (p, rec) = sample(vec![ok(first), response(vec![], "")]);
            let shared = Arc::new(Mutex::new(p));
            let mut host = Host::new(vec![Box::new(PageHost(shared.clone()))], demo::NOW).unwrap();
            host.enable_journal();
            let mut all = vec![];
            let mut replies = vec![];
            for (channel, root) in [(demo::TEAM, ROOT), (demo::SVC, OTHER)] {
                let started = host.update(Event::Msg {
                    plugin: 0,
                    msg: Box::new(Command::Page(channel, root)),
                });
                let current = records(&started);
                let (task_id, semantic, task) = tracked(started);
                let captured = current
                    .iter()
                    .find_map(|record| match record {
                        JournalRecord::Begin { id, descriptor, .. } if id == &semantic => {
                            Some(descriptor.clone())
                        }
                        _ => None,
                    })
                    .unwrap();
                scope(&captured, TEAM, channel, root);
                all.extend(current);
                replies.push((task_id, semantic, captured, run(task)));
            }
            assert_eq!(rec.requests().len(), 2);
            if stale {
                let mut p = shared.lock().unwrap();
                let mut me = p.data.me.clone().unwrap();
                me.team_id = "T000NEXT1".into();
                p.install_me(me);
                // Old completions must not clear fresh loading/error state.
                for fetched in p.threads.values_mut() {
                    fetched.loading = true;
                    fetched.error = Some("fresh error".into());
                }
                p.sync.reading = 7;
            }
            let before = held(&shared.lock().unwrap());
            for (task_id, semantic, captured, response) in replies.into_iter().rev() {
                let is_first = captured.subjects[0].key.id == format!("{}/{ROOT}", demo::TEAM);
                let expected = if is_first {
                    first_outcome
                } else {
                    Outcome::Succeeded
                };
                let result = deliver(&mut host, task_id.clone(), semantic.clone(), response);
                let got = records(&result);
                assert_eq!(terminals(&got, &task_id), [expected]);
                assert_eq!(terminals(&got, &semantic), [expected]);
                let mut skipped = 0;
                for record in &got {
                    if let JournalRecord::Begin {
                        id,
                        descriptor,
                        parent,
                    } = record
                    {
                        if descriptor.kind.as_str().starts_with(PREFIX)
                            || descriptor.kind.as_str() == "slack.read.apply.skipped.stale"
                        {
                            assert_eq!(parent.as_ref(), Some(&task_id));
                            assert_eq!(descriptor.subjects, captured.subjects);
                        }
                        if descriptor.kind.as_str() == "slack.read.apply.skipped.stale" {
                            skipped += 1;
                            assert_eq!(descriptor.metadata.count, Some(1));
                            assert_eq!(terminals(&got, id), [Outcome::Blocked]);
                        }
                    }
                }
                assert_eq!(skipped, usize::from(stale));
                if stale {
                    assert_eq!(held(&shared.lock().unwrap()), before);
                    assert!(result.iter().all(|e| matches!(e, HostEffect::Journal(_))));
                }
                all.extend(got);
            }
            persist_and_check(&all);
            assert_eq!(rec.requests().len(), 2);
        }
    }
}

#[test]
fn combined_cutoff_and_incomplete_counts_have_distinct_units() {
    let effects = page_thread(
        &page_thread_action(TEAM, demo::TEAM, ROOT),
        &Ok(MessagesRead {
            messages: vec![Message::default(); 3],
            evidence: ListEvidence {
                truncated: true,
                incomplete_pages: 2,
            },
        }),
    );
    report(&effects, Outcome::Partial);
    count(&effects, "messages", 3, Outcome::Partial, true);
    count(&effects, "lists.truncated", 1, Outcome::Partial, true);
    count(&effects, "pages.incomplete", 2, Outcome::Partial, false);
}

#[path = "page_open_tests.rs"]
mod open_tests;
