//! No live transport: dispatched tasks are dropped, except explicit empty-token
//! failures which stop before constructing a client. Replies are recorded data.

use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::journal::{ActionId, Outcome};
use radar_core::plugin::{Effect, Msg, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Host, Plugin, writes};

use super::*;
use crate::client::recorded::block;
use crate::mr_actions::{self, ActMsg, Acting};
use crate::settings::{self, ConnRow, Field};
use crate::write::{self, Change, WroteMsg};
use crate::{demo, pages};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const OTHER_URL: &str = "https://other.example";
const SKIPPED: &str = "gitlab.write.refresh.skipped.stale";

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn note() -> Change {
    Change::Note {
        project: PROJECT.into(),
        iid: IID,
        text: "PRIVATE_ORIGINAL_TEXT".into(),
    }
}

fn sent(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
}

fn skipped_count(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| matches!(e, Effect::Observe { action, outcome: Outcome::Blocked }
        if action.kind.as_str() == SKIPPED && action.metadata.count == Some(1) && action.subjects.is_empty())).count()
}

fn edit_connection(p: &mut GitlabPlugin, field: Field, text: &str) {
    p.section_rows[0] = settings::conn_rows(p)
        .iter()
        .position(|row| *row == ConnRow::Field(field))
        .unwrap();
    settings::on_menu(p, &format!("conn:{}", field.label()), "edit");
    for _ in 0..100 {
        settings::key(p, "connection", key(KeyCode::Backspace));
    }
    p.paste(text);
    settings::key(p, "connection", key(KeyCode::Enter));
    assert!(p.edit.is_none(), "setting accepted");
}

#[test]
fn configure_advances_only_connection_changes_and_keeps_held_state() {
    let mut p = demo::sample();
    let _held_id = p.recovery.test_keep("PRIVATE_HELD_TEXT");
    start_fresh(&mut p, PROJECT, IID, write::Target::Note);
    write::compose_paste(&mut p, "PRIVATE_NEW_COMPOSER");
    let before = p.connection_stamp();
    let held = p.snapshot();
    let drafts = p.recovery.records.clone();
    let cfg = p.cfg.clone();
    let mut changed = cfg.clone();
    changed.url = OTHER_URL.into();
    p.configure(Some(&toml::Value::try_from(&changed).unwrap()))
        .unwrap();
    let second = p.connection_stamp();
    assert_ne!(before, second);
    assert_eq!(p.snapshot(), held);
    assert_eq!(p.recovery.records, drafts);
    assert_eq!(p.compose.as_ref().unwrap().text(), "PRIVATE_NEW_COMPOSER");
    p.configure(Some(&toml::Value::try_from(&changed).unwrap()))
        .unwrap();
    assert_eq!(p.connection_stamp(), second);
    p.configure(Some(&toml::Value::try_from(&cfg).unwrap()))
        .unwrap();
    assert!(!p.is_current_connection(before), "A-B-A is still stale");
    assert!(!p.is_current_connection(second));
    let third = p.connection_stamp();
    let mut changed = p.cfg.clone();
    changed.base = "different".into();
    changed.sources.clear();
    changed.refresh_secs += 10;
    changed.side_by_side = !changed.side_by_side;
    p.configure(Some(&toml::Value::try_from(&changed).unwrap()))
        .unwrap();
    assert_eq!(
        p.connection_stamp(),
        third,
        "sources and appearance are not identity"
    );
    changed.token = "env:PRIVATE_OTHER_REFERENCE".into();
    p.configure(Some(&toml::Value::try_from(&changed).unwrap()))
        .unwrap();
    assert_ne!(p.connection_stamp(), third);
    assert!(!p.snapshot().unwrap().to_string().contains("connection"));
    assert!(!p.config().unwrap().to_string().contains("generation"));
    let last = p.connection_stamp();
    assert!(
        p.configure(Some(&toml::Value::String("invalid".into())))
            .is_err()
    );
    assert_eq!(p.connection_stamp(), last);
    p.configure(None).unwrap();
    assert_ne!(
        p.connection_stamp(),
        last,
        "default connection restoration is covered"
    );
}

#[test]
fn settings_edits_and_token_offers_use_the_same_generation() {
    let mut p = demo::sample();
    let first = p.connection_stamp();
    let original_url = p.cfg.url.clone();
    edit_connection(&mut p, Field::Url, &original_url);
    assert_eq!(p.connection_stamp(), first);
    edit_connection(&mut p, Field::Url, OTHER_URL);
    let second = p.connection_stamp();
    assert_ne!(second, first);
    edit_connection(&mut p, Field::Url, &original_url);
    assert!(!p.is_current_connection(first));
    let third = p.connection_stamp();
    edit_connection(&mut p, Field::Token, "glab");
    let fourth = p.connection_stamp();
    assert_ne!(fourth, third);
    edit_connection(&mut p, Field::Token, "glab");
    assert_eq!(p.connection_stamp(), fourth);
    edit_connection(&mut p, Field::Base, "another/group");
    edit_connection(&mut p, Field::Refresh, "90");
    assert_eq!(p.connection_stamp(), fourth);
    p.env = Some(vec![(
        "PRIVATE_GITLAB_TOKEN".into(),
        "glpat-not.a.real.token.1234".into(),
    )]);
    p.me.clear();
    settings::on_menu(&mut p, "offer:0", "use");
    assert_eq!(p.cfg.token, "env:PRIVATE_GITLAB_TOKEN");
    assert_ne!(p.connection_stamp(), fourth);
}

#[test]
#[should_panic(expected = "GitLab connection generation exhausted")]
fn connection_generations_never_wrap() {
    let mut p = demo::sample();
    p.connection = ConnectionStamp(u64::MAX);
    p.replace_connection(OTHER_URL.into(), "glab".into());
}

#[test]
fn actual_dispatch_tasks_capture_the_original_generation_without_network() {
    for what in ["comment", "rebase", "merge"] {
        let mut p = demo::sample();
        // An empty literal fails before Client::new. No environment or glab read.
        p.replace_connection(p.cfg.url.clone(), String::new());
        let connection = p.connection_stamp();
        let effects = if what == "comment" {
            write::ask(&mut p, note());
            write::on_menu(&mut p, "gl-write", "yes").unwrap()
        } else {
            mr_actions::start(&mut p, PROJECT, IID, what)
        };
        let task = effects
            .into_iter()
            .find_map(|e| match e {
                Effect::Spawn(task) => Some(task),
                _ => None,
            })
            .unwrap();
        p.replace_connection(OTHER_URL.into(), "glab".into());
        let message = block(task);
        let message: Msg = if what == "comment" {
            let m = message.downcast::<WroteMsg>().unwrap();
            assert_eq!(m.connection, connection);
            assert!(
                m.result
                    .as_ref()
                    .unwrap_err()
                    .contains("no GitLab token configured")
            );
            m
        } else {
            let m = message.downcast::<ActMsg>().unwrap();
            assert_eq!(m.connection, connection);
            assert_eq!(m.outcome, Outcome::Failed);
            assert!(
                m.result
                    .as_ref()
                    .unwrap_err()
                    .contains("no GitLab token configured")
            );
            m
        };
        let effects = p.on_msg(message);
        assert!(!sent(&effects));
        assert_eq!(skipped_count(&effects), 1);
        assert!(matches!(effects[0], Effect::Report(outcome)
            if outcome == if what == "comment" { Outcome::Unknown } else { Outcome::Failed }));
    }
}

#[test]
fn stale_writes_keep_original_results_text_and_newer_composer_without_refreshing() {
    let changes = [
        note(),
        Change::Reply {
            project: PROJECT.into(),
            iid: IID,
            discussion: "ab12".into(),
            place: "PRIVATE_PLACE".into(),
            text: "PRIVATE_REPLY".into(),
        },
        Change::Resolve {
            project: PROJECT.into(),
            iid: IID,
            discussion: "ab12".into(),
            place: "PRIVATE_PLACE".into(),
            resolve: true,
        },
        Change::Approve {
            project: PROJECT.into(),
            iid: IID,
            approve: true,
            sha: None,
        },
    ];
    for change in changes {
        for result in [
            Ok("PRIVATE_REMOTE_SUCCESS".into()),
            Err("PRIVATE_REMOTE_ERROR".into()),
        ] {
            let mut p = demo::sample();
            let connection = p.connection_stamp();
            write::ask(&mut p, change.clone());
            assert!(sent(&write::on_menu(&mut p, "gl-write", "yes").unwrap()));
            p.replace_connection(OTHER_URL.into(), p.cfg.token.clone());
            start_fresh(&mut p, PROJECT, IID, write::Target::Note);
            write::compose_paste(&mut p, "PRIVATE_NEW_COMPOSER");
            let held_id = p.recovery.test_keep("PRIVATE_HELD_TEXT");
            let held = p.snapshot();
            let pages = format!("{:?}", p.mr_pages);
            let sync = format!("{:?}", p.sync);
            let success = result.is_ok();
            let effects = p.on_msg(Box::new(WroteMsg {
                operation: 1,
                connection,
                change: change.clone(),
                result,
            }));
            assert_eq!(p.snapshot(), held);
            assert_eq!(format!("{:?}", p.mr_pages), pages);
            assert_eq!(format!("{:?}", p.sync), sync);
            assert_eq!(p.compose.as_ref().unwrap().text(), "PRIVATE_NEW_COMPOSER");
            assert_eq!(p.recovery.records[&held_id].original, "PRIVATE_HELD_TEXT");
            assert!(!sent(&effects));
            assert_eq!(skipped_count(&effects), 1);
            assert!(matches!(effects[0], Effect::Report(outcome)
                if outcome == if success { Outcome::Succeeded } else { Outcome::Unknown }));
            assert!(effects.iter().any(|e| matches!(e, Effect::Notify(m)
                if m.contains(if success { "PRIVATE_REMOTE_SUCCESS" } else { "PRIVATE_REMOTE_ERROR" }))));
            assert!(!matches!(p.acting[&change.key()].state, Acting::Running));
            if !success && let Some(text) = change.text() {
                assert!(
                    p.recovery
                        .records
                        .values()
                        .any(|draft| draft.original == text)
                );
            }
        }
    }
}

#[test]
fn stale_merge_and_rebase_results_keep_all_original_outcomes() {
    for what in ["rebase", "merge"] {
        for outcome in [
            Outcome::Succeeded,
            Outcome::Failed,
            Outcome::Unknown,
            Outcome::Blocked,
        ] {
            let mut p = demo::sample();
            let connection = p.connection_stamp();
            assert!(sent(&mr_actions::start(&mut p, PROJECT, IID, what)));
            p.replace_connection(OTHER_URL.into(), p.cfg.token.clone());
            let held = p.snapshot();
            let result = if outcome == Outcome::Succeeded {
                Ok("PRIVATE_REMOTE_SUCCESS".into())
            } else {
                Err("PRIVATE_REMOTE_ERROR".into())
            };
            let effects = p.on_msg(Box::new(ActMsg {
                connection,
                key: pages::mr_key(PROJECT, IID),
                what,
                result,
                outcome,
            }));
            assert!(matches!(effects[0], Effect::Report(got) if got == outcome));
            assert_eq!(skipped_count(&effects), 1);
            assert!(!sent(&effects));
            assert_eq!(p.snapshot(), held);
            assert!(!matches!(
                p.acting[&pages::mr_key(PROJECT, IID)].state,
                Acting::Running
            ));
        }
    }
}

#[test]
fn unchanged_connection_and_source_only_edits_still_refresh_after_completion() {
    for what in ["comment", "rebase", "merge"] {
        let mut p = demo::sample();
        let connection = p.connection_stamp();
        let mut cfg = p.cfg.clone();
        cfg.sources.remove(0);
        cfg.base = "other".into();
        cfg.hide_files = !cfg.hide_files;
        p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
            .unwrap();
        let message: Msg = if what == "comment" {
            write::ask(&mut p, note());
            assert!(sent(&write::on_menu(&mut p, "gl-write", "yes").unwrap()));
            Box::new(WroteMsg {
                operation: 1,
                connection,
                change: note(),
                result: Ok("posted".into()),
            })
        } else {
            Box::new(ActMsg {
                connection,
                key: pages::mr_key(PROJECT, IID),
                what,
                result: Ok("done".into()),
                outcome: Outcome::Succeeded,
            })
        };
        let effects = p.on_msg(message);
        assert!(sent(&effects));
        assert_eq!(skipped_count(&effects), 0);
        assert!(p.mr_pages[&pages::mr_key(PROJECT, IID)].loading);
        assert!(p.sync.in_flight > 0);
    }
}

#[test]
fn mode_refusal_keeps_text_and_does_not_invent_stale_refresh_work() {
    let prior = writes::mode();
    writes::set_mode(writes::Mode::ReadOnly, None);
    for what in ["comment", "rebase", "merge"] {
        let mut p = demo::sample();
        p.replace_connection(OTHER_URL.into(), "glab".into());
        let effects = if what == "comment" {
            write::ask(&mut p, note());
            write::on_menu(&mut p, "gl-write", "yes").unwrap()
        } else {
            mr_actions::start(&mut p, PROJECT, IID, what)
        };
        assert!(!sent(&effects));
        assert_eq!(skipped_count(&effects), 0);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Report(Outcome::Blocked)))
        );
        if what == "comment" {
            assert!(
                p.recovery
                    .records
                    .values()
                    .any(|record| record.original == "PRIVATE_ORIGINAL_TEXT")
            );
        }
    }
    writes::set_mode(prior.0, prior.1);
}

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}

fn tasks(effects: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect()
}

fn host_edit(host: &mut Host, field: Field, text: &str) {
    assert_eq!(host.view, View::Settings);
    host.focus = Pane::Middle;
    // The host entered connection on its first row; navigate to the requested row.
    for _ in 0..Field::ALL.iter().position(|f| *f == field).unwrap() {
        host.update(Event::Key(key(KeyCode::Down)));
    }
    host.update(Event::Key(key(KeyCode::Char('e'))));
    assert!(host.capturing());
    for _ in 0..100 {
        host.update(Event::Key(key(KeyCode::Backspace)));
    }
    host.update(Event::Paste(text.into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(!host.capturing());
    assert_eq!(
        host.menu.as_ref().unwrap().menu.id,
        crate::settings_checks::Target::Auth.menu()
    );
    host.update(Event::Key(key(KeyCode::Char('n'))));
}

#[test]
fn real_host_allows_settings_after_confirmation_and_preserves_correlated_write_outcome() {
    for (comment, field, new_value) in
        [(true, Field::Url, OTHER_URL), (false, Field::Token, "glab")]
    {
        let mut p = demo::sample();
        let original_url = p.cfg.url.clone();
        let connection = p.connection_stamp();
        if comment {
            start_fresh(&mut p, PROJECT, IID, write::Target::Note);
            write::compose_paste(&mut p, "PRIVATE_ORIGINAL_TEXT");
        } else {
            pages::push(
                &mut p,
                pages::Kind::Mr {
                    project: "acme/team-a/web".into(),
                    iid: 187,
                },
            );
        }
        p.section_rows[0] = 0;
        let mut host = Host::new(vec![Box::new(p)], 1).unwrap();
        host.view = View::Screen(0);
        host.focus = Pane::Middle;
        host.section = host
            .sections
            .iter()
            .position(|(_, s)| s.id == "connection" && s.group == "gitlab")
            .unwrap();
        host.enable_journal();
        let review = if comment {
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        } else {
            key(KeyCode::Char('m'))
        };
        let asked = host.update(Event::Key(review));
        assert!(host.menu.is_some());
        host.update(Event::Key(key(KeyCode::Char(','))));
        assert_eq!(
            host.view,
            View::Screen(0),
            "unconfirmed modal blocks settings"
        );
        let dispatched = host.update(Event::Key(key(KeyCode::Char('y'))));
        let pending = tasks(&dispatched);
        assert_eq!(pending.len(), 1);
        assert!(host.menu.is_none());
        assert!(!host.capturing());
        host.update(Event::Key(key(KeyCode::Char(','))));
        host_edit(&mut host, field, new_value);
        let cfg = host.plugins[0].config().unwrap();
        assert_eq!(
            cfg[if comment { "url" } else { "token" }].as_str(),
            Some(new_value)
        );
        let message: Msg = if comment {
            Box::new(WroteMsg {
                operation: 1,
                connection,
                change: note(),
                result: Ok("PRIVATE_REMOTE_SUCCESS".into()),
            })
        } else {
            Box::new(ActMsg {
                connection,
                key: "acme/team-a/web!187".into(),
                what: "merge",
                result: Ok("PRIVATE_REMOTE_SUCCESS".into()),
                outcome: Outcome::Succeeded,
            })
        };
        let completed = host.update(Event::Tracked {
            action: pending[0].0.clone(),
            semantic: Some(pending[0].1.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        assert!(tasks(&completed).is_empty());
        assert!(
            host.message
                .as_ref()
                .unwrap()
                .text
                .contains("PRIVATE_REMOTE_SUCCESS")
        );
        let original = records(&asked)
            .into_iter()
            .find_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. } if id == pending[0].1 => {
                    Some(descriptor)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            original.subjects[0].key.instance,
            format!("{original_url}/")
        );
        let recorded = records(&completed);
        for id in [&pending[0].0, &pending[0].1] {
            let terminal: Vec<_> = recorded
                .iter()
                .filter_map(|r| match r {
                    JournalRecord::Outcome {
                        id: got, outcome, ..
                    } if got == id && outcome.is_terminal() => Some(*outcome),
                    _ => None,
                })
                .collect();
            assert_eq!(
                terminal,
                [Outcome::Succeeded],
                "original result is not blocked"
            );
        }
        let skipped = recorded
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                    ..
                } if descriptor.kind.as_str() == SKIPPED => {
                    assert_eq!(parent.as_ref(), Some(&pending[0].0));
                    assert!(descriptor.subjects.is_empty());
                    assert_eq!(descriptor.metadata.count, Some(1));
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        assert!(recorded.iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Blocked, .. } if id == skipped)));
        let exported = format!("{recorded:?}");
        assert!(!exported.contains("PRIVATE_"));
        assert!(!exported.contains(OTHER_URL));
        assert!(!exported.contains("glab"));
        // All task/save effects remain unexecuted. The real host path, rather
        // than a replayed menu id after config mutation, establishes reachability.
    }
}

fn start_fresh(p: &mut GitlabPlugin, project: &str, iid: u64, target: write::Target) {
    let fx = write::start(p, project, iid, target);
    if let Some(Effect::Menu(menu)) = fx.first() {
        let id = menu.id.clone();
        p.on_menu(&id, "new");
    }
}
