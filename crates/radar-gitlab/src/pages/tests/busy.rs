//! Busy confirmations never consume newer text. All tasks remain unpolled:
//! completions below are recorded data, with no token resolution or network.

use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::journal::{ActionId, Outcome as JournalOutcome};
use radar_core::writes;

use super::*;
use crate::mr_actions::Acting;
use crate::write::{self, Change, LineAt, Target, WroteMsg};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const FIRST: &str = "PRIVATE_FIRST_COMMENT";
const SECOND: &str = "PRIVATE_SECOND_COMMENT  \nsecond line  \n  ";

struct ModeGuard((writes::Mode, Option<String>));

impl ModeGuard {
    fn edit() -> Self {
        let old = writes::mode();
        writes::set_mode(writes::Mode::Edit, None);
        Self(old)
    }
}

impl Drop for ModeGuard {
    fn drop(&mut self) {
        writes::set_mode(self.0.0, self.0.1.take());
    }
}

fn first() -> Change {
    Change::Note {
        project: PROJECT.into(),
        iid: IID,
        text: FIRST.into(),
    }
}

fn sent(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::Spawn(_) | Effect::SpawnAction { .. }))
}

#[test]
fn busy_note_line_and_reply_keep_exact_composer_pending_change_and_first_result() {
    let _mode = ModeGuard::edit();
    let targets = [
        Target::Note,
        Target::Line(LineAt {
            new_path: "src/a.rs".into(),
            old_path: "src/a.rs".into(),
            old: Some(12),
            new: Some(14),
            refs: crate::model::DiffRefs {
                base: "b".into(),
                start: "s".into(),
                head: "h".into(),
            },
        }),
        Target::Reply {
            discussion: "ab12".into(),
            place: "a.rs:14".into(),
        },
    ];
    for target in targets {
        for success in [true, false] {
            let mut p = sample();
            let connection = p.connection_stamp();
            write::ask(&mut p, first());
            assert!(sent(&p.on_menu("gl-write", "yes")));
            let held_id = p.recovery.test_keep("PRIVATE_SAVED_TEXT");
            start_fresh(&mut p, PROJECT, IID, target.clone());
            p.paste(SECOND);
            assert!(!sent(&ctrl_s(&mut p)));
            let pending = p.change.clone();
            let drafts = p.recovery.records.clone();
            let acting = format!("{:?}", p.acting);
            for _ in 0..2 {
                let effects = p.on_menu("gl-write", "yes");
                assert!(matches!(&effects[..], [
                    Effect::Report(JournalOutcome::Confirmed),
                    Effect::Report(JournalOutcome::Blocked), Effect::Notify(text)
                ] if text == "GitLab has not answered the last change yet"));
                assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
                assert_eq!(p.change, pending);
                assert_eq!(p.recovery.records, drafts);
                assert_eq!(format!("{:?}", p.acting), acting);
                assert!(p.capturing());
            }
            press(&mut p, Key::Char('!'));
            p.paste("PRIVATE_PASTE");
            let edited = format!("{SECOND}!PRIVATE_PASTE");
            assert_eq!(p.compose.as_ref().unwrap().text(), edited);
            let effects = write::on_wrote(
                &mut p,
                WroteMsg {
                    operation: 1,
                    connection,
                    change: first(),
                    result: if success {
                        Ok("PRIVATE_SUCCESS".into())
                    } else {
                        Err("PRIVATE_ERROR".into())
                    },
                },
            );
            assert!(!sent(&effects), "completion never retries the newer draft");
            assert!(matches!(effects[0], Effect::Report(outcome)
                if outcome == if success { JournalOutcome::Succeeded } else { JournalOutcome::Unknown }));
            assert_eq!(p.compose.as_ref().unwrap().text(), edited);
            assert_eq!(p.change, pending);
            assert_eq!(p.recovery.records[&held_id].original, "PRIVATE_SAVED_TEXT");
            assert!(!sent(&ctrl_s(&mut p)), "fresh review alone sends nothing");
            assert_eq!(
                p.change.as_ref().and_then(Change::text),
                Some(edited.as_str())
            );
            assert!(sent(&p.on_menu("gl-write", "yes")));
            assert!(p.compose.is_none() && p.change.is_none());
            assert!(
                !sent(&p.on_menu("gl-write", "yes")),
                "accepted snapshot is consumed once"
            );
        }
    }
}

#[test]
fn busy_confirmation_does_not_block_a_different_merge_request() {
    let _mode = ModeGuard::edit();
    let mut p = sample();
    write::ask(&mut p, first());
    assert!(sent(&p.on_menu("gl-write", "yes")));
    write::start(&mut p, PROJECT, IID + 1, Target::Note);
    p.paste(SECOND);
    ctrl_s(&mut p);
    assert!(sent(&p.on_menu("gl-write", "yes")));
    assert!(p.compose.is_none() && p.change.is_none());
    for iid in [IID, IID + 1] {
        assert_eq!(p.acting[&mr_key(PROJECT, iid)].state, Acting::Running);
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

fn tasks(effects: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
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

fn key(code: Key) -> Event {
    crate::key_event(Chord::new(code))
}

fn review() -> Event {
    crate::key_event(Chord::ctrl('s'))
}

#[test]
fn host_busy_confirmation_returns_to_editing_and_keeps_original_action_outcome_private() {
    let _mode = ModeGuard::edit();
    for success in [true, false] {
        let mut p = mr_page();
        p.pages[0].tab = mr::THREADS;
        let connection = p.connection_stamp();
        let mut host = radar_core::Host::new(vec![Box::new(p)], 1).unwrap();
        host.view = View::Screen(0);
        host.focus = Pane::Middle;
        host.enable_journal();
        let mut journal = Vec::new();
        journal.extend(records(&host.update(key(Key::Char('c')))));
        journal.extend(records(&host.update(Event::Paste(FIRST.into()))));
        journal.extend(records(&host.update(review())));
        let dispatched = host.update(key(Key::Char('y')));
        journal.extend(records(&dispatched));
        let pending = tasks(&dispatched);
        assert_eq!(pending.len(), 1);
        assert!(!host.capturing() && host.menu.is_none());
        journal.extend(records(&host.update(key(Key::Char('c')))));
        journal.extend(records(&host.update(key(Key::Char('n')))));
        journal.extend(records(&host.update(Event::Paste(SECOND.into()))));
        let preview = host.update(review());
        let newer = records(&preview)
            .into_iter()
            .find_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "gitlab.comment.post" =>
                {
                    Some(id)
                }
                _ => None,
            })
            .unwrap();
        journal.extend(records(&preview));
        let refused = host.update(key(Key::Char('y')));
        journal.extend(records(&refused));
        assert!(tasks(&refused).is_empty(), "no second send task");
        assert!(host.menu.is_none() && host.capturing());
        assert!(
            host.message
                .as_ref()
                .unwrap()
                .text
                .contains("has not answered")
        );
        let refused_records = records(&refused);
        assert!(refused_records.iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, outcome: JournalOutcome::Blocked, .. } if id == &newer)));
        assert!(!refused_records.iter().any(|record| matches!(record,
            JournalRecord::Outcome { id, outcome, .. }
                if (id == &pending[0].0 || id == &pending[0].1) && outcome.is_terminal())));
        let drawn = crate::tests::shown(&host, 160, 40);
        assert!(drawn.contains("PRIVATE_SECOND_COMMENT"), "{drawn}");
        journal.extend(records(&host.update(key(Key::Char('!')))));
        journal.extend(records(&host.update(Event::Paste("PRIVATE_PASTE".into()))));
        let completed = host.update(Event::Tracked {
            action: pending[0].0.clone(),
            semantic: Some(pending[0].1.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(WroteMsg {
                    operation: 1,
                    connection,
                    change: first(),
                    result: if success {
                        Ok("PRIVATE_SUCCESS".into())
                    } else {
                        Err("PRIVATE_ERROR".into())
                    },
                }),
            }),
        });
        journal.extend(records(&completed));
        assert!(host.capturing() && host.menu.is_none());
        let completed_records = records(&completed);
        for id in [&pending[0].0, &pending[0].1] {
            let outcomes: Vec<_> = completed_records
                .iter()
                .filter_map(|record| match record {
                    JournalRecord::Outcome {
                        id: got, outcome, ..
                    } if got == id && outcome.is_terminal() => Some(*outcome),
                    _ => None,
                })
                .collect();
            assert_eq!(
                outcomes,
                [if success {
                    JournalOutcome::Succeeded
                } else {
                    JournalOutcome::Unknown
                }]
            );
        }
        assert!(!completed_records.iter().any(|record| matches!(record,
            JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "gitlab.comment.post")),
            "completion may refresh but cannot automatically submit B");
        let fresh_review = host.update(review());
        journal.extend(records(&fresh_review));
        assert!(tasks(&fresh_review).is_empty());
        let menu = &host.menu.as_ref().unwrap().menu;
        let shown: Vec<_> = menu
            .items
            .iter()
            .filter(|item| item.note)
            .map(|item| item.label.clone())
            .collect();
        assert_eq!(
            shown,
            [
                "PRIVATE_SECOND_COMMENT  ",
                "second line  ",
                "  !PRIVATE_PASTE"
            ]
        );
        let second_dispatch = host.update(key(Key::Char('y')));
        journal.extend(records(&second_dispatch));
        assert_eq!(
            tasks(&second_dispatch).len(),
            1,
            "only fresh confirmation submits B"
        );
        assert!(!host.capturing());
        assert!(!format!("{journal:?}").contains("PRIVATE_"));
        // Neither original/second send nor automatic read tasks are ever polled.
    }
}

fn start_fresh(p: &mut GitlabPlugin, project: &str, iid: u64, target: write::Target) {
    let fx = write::start(p, project, iid, target);
    if let Some(Effect::Menu(menu)) = fx.first() {
        let id = menu.id.clone();
        p.on_menu(&id, "new");
    }
}

#[test]
fn host_escape_from_nontext_review_allows_blank_new_and_restored_comments() {
    for action in ['a', 'd'] {
        for recovery in ["blank", "new", "edit"] {
            let mut p = mr_page();
            p.pages[0].tab = mr::THREADS;
            if action == 'd' {
                p.pages[0].row.set(1);
            }
            draw(&p, 160, 40); // Review keys consume the painted thread row.
            if recovery != "blank" {
                press(&mut p, Key::Char('c'));
                p.paste(SECOND);
                ctrl_s(&mut p);
                p.on_menu("gl-write", "no");
            }
            let mut host = radar_core::Host::new(vec![Box::new(p)], 1).unwrap();
            host.view = View::Screen(0);
            host.focus = Pane::Middle;
            host.update(key(Key::Char(action)));
            assert!(host.menu.is_some(), "{action}: nontext review open");
            host.update(key(Key::Esc));
            assert!(host.menu.is_none() && !host.capturing());
            host.update(key(Key::Char('c')));
            if recovery != "blank" {
                assert!(
                    host.menu
                        .as_ref()
                        .unwrap()
                        .menu
                        .title
                        .contains("kept comments")
                );
                if recovery == "new" {
                    host.update(key(Key::Char('n')));
                } else {
                    host.update(key(Key::Enter));
                    host.update(key(Key::Char('e')));
                }
            }
            assert!(
                host.capturing() && host.menu.is_none(),
                "{action}/{recovery}: composer opens after abandoned review"
            );
            // The abandoned nontext snapshot must not survive the new editor.
            assert!(!sent(&host.plugins[0].on_menu("gl-write", "yes")));
            if recovery == "edit" {
                host.update(review());
                let lines: Vec<_> = host
                    .menu
                    .as_ref()
                    .unwrap()
                    .menu
                    .items
                    .iter()
                    .filter(|i| i.note)
                    .map(|i| i.label.as_str())
                    .collect();
                assert_eq!(lines.join("\n"), SECOND.trim_end());
            } else {
                host.update(Event::Paste("PRIVATE_NEW_AFTER_ESCAPE".into()));
                host.update(review());
                assert!(
                    host.menu
                        .as_ref()
                        .unwrap()
                        .menu
                        .items
                        .iter()
                        .any(|i| i.label == "PRIVATE_NEW_AFTER_ESCAPE")
                );
            }
        }
    }
}
