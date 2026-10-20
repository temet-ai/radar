//! Busy confirmation regressions. Spawned futures are never polled.

use super::*;
use crate::{demo, tests};
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use serde_json::json;

const FIRST: &str = "SECRET_FIRST";
const SECOND: &str = "  SECRET_SECOND\n\nlast  ";

fn bug() -> IssueType {
    IssueType {
        id: "10004".into(),
        name: "Bug".into(),
        subtask: false,
    }
}

fn comment(site: &str, key: &str, text: &str) -> Change {
    Change::Comment {
        site: site.into(),
        key: key.into(),
        text: text.into(),
    }
}

fn create(site: &str, project: &str, description: &str) -> Change {
    Change::Create {
        site: site.into(),
        project: project.into(),
        kind: bug(),
        summary: "SECRET_SUMMARY".into(),
        description: description.into(),
    }
}

fn editor(change: &Change, text: &str) -> Compose {
    let draft = match change {
        Change::Comment { key, .. } => Draft::Comment { key: key.clone() },
        Change::Create {
            project,
            kind,
            summary,
            ..
        } => Draft::Description {
            project: project.clone(),
            kind: kind.clone(),
            summary: summary.clone(),
        },
        _ => unreachable!(),
    };
    Compose::new(change.site(), draft, text)
}

#[test]
fn busy_confirmation_keeps_exact_editor_cursor_intent_and_existing_drafts() {
    for change in [
        comment("acme", "ABC-12", SECOND.trim_end()),
        create("acme", "ABC", SECOND.trim_end()),
    ] {
        let mut p = demo::sample();
        p.compose = Some(editor(&change, SECOND));
        let compose = p.compose.as_mut().unwrap();
        compose
            .composer
            .key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let cursor = compose.composer.input.cursor();
        let draft = compose.draft.clone();
        p.change = Some(change.clone());
        let pending = Sent {
            dispatch: None,
            words: "original pending request".into(),
            result: None,
        };
        p.sent.insert(sent_id(&change), pending.clone());
        let record = Record::capture(&mut p, &comment("acme", "OTHER-1", "SECRET_SAVED")).unwrap();
        p.recovery.reserve(record).unwrap();
        let drafts = p.recovery.records.clone();
        let expected = match change.journal(&p) {
            Effect::Journal(action) => action,
            _ => unreachable!(),
        };

        let reviewed = p.change.clone().unwrap();
        let _ = ask(&mut p, reviewed);
        let menu = p.dialogs.menu_id();
        let effects = on_menu(&mut p, &menu, "yes").unwrap();
        assert!(matches!(&effects[..], [
            Effect::Journal(action),
            Effect::Report(JournalOutcome::Confirmed),
            Effect::Report(JournalOutcome::Blocked),
            Effect::Notify(notice),
        ] if format!("{action:?}") == format!("{expected:?}") && notice == "Jira has not answered the last change yet"));
        assert_eq!(p.change.as_ref(), Some(&change));
        let compose = p.compose.as_ref().unwrap();
        assert_eq!(compose.text(), SECOND);
        assert_eq!(compose.site, change.site());
        assert_eq!(compose.draft, draft);
        assert_eq!(compose.composer.input.cursor(), cursor);
        assert_eq!(p.sent[&sent_id(&change)].words, pending.words);
        assert_eq!(p.sent[&sent_id(&change)].result, pending.result);
        assert_eq!(p.recovery.records, drafts);

        // Direct callers still encounter the original final dispatch guard.
        let effects = send(&mut p, change);
        assert!(matches!(
            &effects[..],
            [
                Effect::Journal(_),
                Effect::Report(JournalOutcome::Confirmed),
                Effect::Report(JournalOutcome::Blocked),
                Effect::Notify(_),
            ]
        ));
    }
}

#[test]
fn independent_ticket_project_and_site_or_finished_write_can_send() {
    for (original, next, finished) in [
        (
            comment("acme", "ABC-12", FIRST),
            comment("acme", "ABC-13", SECOND),
            false,
        ),
        (
            create("acme", "ABC", FIRST),
            create("acme", "DEF", SECOND),
            false,
        ),
        (
            comment("acme", "ABC-12", FIRST),
            comment("other", "ABC-12", SECOND),
            false,
        ),
        (
            create("acme", "ABC", FIRST),
            create("other", "ABC", SECOND),
            false,
        ),
        (
            comment("acme", "ABC-12", FIRST),
            comment("acme", "ABC-12", SECOND),
            true,
        ),
        (
            create("acme", "ABC", FIRST),
            create("acme", "ABC", SECOND),
            true,
        ),
    ] {
        writes::set_mode(writes::Mode::Write, None);
        let mut p = demo::sample();
        let mut other = p.cfg.site("acme").unwrap().clone();
        other.name = "other".into();
        other.url = "https://other.example".into();
        p.cfg.sites.push(other);
        p.sent.insert(
            sent_id(&original),
            Sent {
                dispatch: None,
                words: original.words(),
                result: finished.then(|| Ok("done".into())),
            },
        );
        let next = editor(&next, SECOND).change().unwrap();
        p.compose = Some(editor(&next, SECOND));
        p.change = Some(next.clone());
        let reviewed = p.change.clone().unwrap();
        let _ = ask(&mut p, reviewed);
        let menu = p.dialogs.menu_id();
        let effects = on_menu(&mut p, &menu, "yes").unwrap();
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::Spawn(_)))
                .count(),
            1
        );
        assert!(p.change.is_none() && p.compose.is_none());
        assert_eq!(p.sent[&sent_id(&next)].result, None);
        if !finished {
            assert_eq!(p.sent[&sent_id(&original)].result, None);
            assert_eq!(p.sent.len(), 2);
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

fn update(host: &mut Host, all: &mut Vec<JournalRecord>, event: Event) -> Vec<HostEffect> {
    let effects = host.update(event);
    all.extend(records(&effects));
    effects
}

fn key(host: &mut Host, all: &mut Vec<JournalRecord>, code: KeyCode) -> Vec<HostEffect> {
    update(
        host,
        all,
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    )
}

fn review(host: &mut Host, all: &mut Vec<JournalRecord>) {
    update(
        host,
        all,
        Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
    );
}

fn paste(host: &mut Host, all: &mut Vec<JournalRecord>, text: &str) {
    update(host, all, Event::Paste(text.into()));
}

fn spawn_contexts(effects: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if matches!(**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect()
}

fn outcomes(records: &[JournalRecord], target: &ActionId) -> Vec<JournalOutcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome { id, outcome, .. } if id == target => Some(*outcome),
            _ => None,
        })
        .collect()
}

fn begin_composer(host: &mut Host, all: &mut Vec<JournalRecord>, is_create: bool, text: &str) {
    key(host, all, KeyCode::Char(if is_create { 'n' } else { 'c' }));
    if host
        .menu
        .as_ref()
        .is_some_and(|m| m.menu.id.starts_with("j-recovery:"))
    {
        key(host, all, KeyCode::Char('n'));
    }
    if is_create {
        let read = tests::jira_mut(host).dialogs.current_read();
        update(
            host,
            all,
            Event::Msg {
                plugin: 0,
                msg: Box::new(JiraMsg::Types {
                    read,
                    site: "acme".into(),
                    project: "ABC".into(),
                    result: Ok(vec![bug()]),
                }),
            },
        );
        key(host, all, KeyCode::Char('1'));
        paste(host, all, "  SECRET_SUMMARY  ");
        key(host, all, KeyCode::Enter);
    }
    paste(host, all, text);
}

fn assert_private_history(all: Vec<JournalRecord>) {
    let dir = std::env::temp_dir().join(format!("radar-jira-busy-{}", ActionId::new().as_str()));
    let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = journal.start_session().unwrap();
    for record in all {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => {
                journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap();
            }
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                journal.record(&id, outcome, metadata).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected history gap"),
        }
    }
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(!serde_json::to_string(&events).unwrap().contains("SECRET"));
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}

fn busy_host_flow(is_create: bool, succeeds: bool) {
    let mut host = tests::hosted();
    host.set_env_mode(Some((writes::Mode::Write, "RADAR_MODE".into())));
    page::open(tests::jira_mut(&mut host), "acme", "ABC-12");
    crate::tests::complete_detail(
        tests::jira_mut(&mut host),
        "acme",
        "ABC-12",
        Ok(tests::detail()),
    );
    host.enable_journal();
    let mut all = Vec::new();

    begin_composer(&mut host, &mut all, is_create, FIRST);
    review(&mut host, &mut all);
    let original = tests::jira_mut(&mut host).change.clone().unwrap();
    let effects = key(&mut host, &mut all, KeyCode::Char('y'));
    let contexts = spawn_contexts(&effects);
    assert_eq!(
        contexts.len(),
        1,
        "original request dispatched once, never polled"
    );
    let (task, original_action) = contexts[0].clone();
    let dispatch = tests::jira_mut(&mut host).sent[&sent_id(&original)].dispatch.clone().unwrap();
    assert!(tests::jira_mut(&mut host).compose.is_none());

    begin_composer(&mut host, &mut all, is_create, SECOND);
    key(&mut host, &mut all, KeyCode::Left);
    let before = tests::jira_mut(&mut host)
        .compose
        .as_ref()
        .unwrap()
        .composer
        .input
        .cursor();
    review(&mut host, &mut all);
    let pending = tests::jira_mut(&mut host).change.clone().unwrap();
    let effects = key(&mut host, &mut all, KeyCode::Char('y'));
    assert!(spawn_contexts(&effects).is_empty());
    assert!(host.menu.is_none());
    assert_eq!(
        host.message.as_deref(),
        Some("Jira has not answered the last change yet")
    );
    let blocked = records(&effects)
        .into_iter()
        .find_map(|record| match record {
            JournalRecord::Outcome {
                id,
                outcome: JournalOutcome::Blocked,
                ..
            } => Some(id),
            _ => None,
        })
        .unwrap();
    assert_ne!(blocked, original_action);
    assert_eq!(
        outcomes(&records(&effects), &blocked),
        [JournalOutcome::Confirmed, JournalOutcome::Blocked]
    );
    let p = tests::jira_mut(&mut host);
    assert_eq!(p.change.as_ref(), Some(&pending));
    assert_eq!(p.compose.as_ref().unwrap().text(), SECOND);
    assert_eq!(p.compose.as_ref().unwrap().composer.input.cursor(), before);

    // The refused editor still captures host shortcut keys and a multiline paste.
    key(&mut host, &mut all, KeyCode::Char('q'));
    paste(&mut host, &mut all, " SECRET_PASTE\nend  ");
    assert!(!host.quit);
    let retained = tests::jira_mut(&mut host).compose.as_ref().unwrap().text();
    assert_eq!(retained, "  SECRET_SECOND\n\nlast q SECRET_PASTE \nend  ");
    let cursor = tests::jira_mut(&mut host)
        .compose
        .as_ref()
        .unwrap()
        .composer
        .input
        .cursor();
    let completion = update(
        &mut host,
        &mut all,
        Event::Tracked {
            action: task,
            semantic: Some(original_action.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(JiraMsg::Wrote {
                    dispatch,
                    change: original.clone(),
                    result: if succeeds {
                        Ok(if is_create {
                            json!({"key": "ABC-31"})
                        } else {
                            Value::Null
                        })
                    } else {
                        Err("SECRET_FAILURE".into())
                    },
                }),
            }),
        },
    );
    assert_eq!(
        outcomes(&records(&completion), &original_action),
        [if succeeds {
            JournalOutcome::Succeeded
        } else {
            JournalOutcome::Unknown
        }]
    );
    assert!(!records(&completion).iter().any(|r| matches!(r,
        JournalRecord::Begin { descriptor, .. }
            if descriptor.kind.as_str() == if is_create { "jira.ticket.create" } else { "jira.comment.post" }
    )), "completion must not retry either write");
    assert_eq!(
        outcomes(&all, &blocked),
        [JournalOutcome::Confirmed, JournalOutcome::Blocked]
    );
    let p = tests::jira_mut(&mut host);
    assert_eq!(p.change.as_ref(), Some(&pending));
    assert_eq!(p.compose.as_ref().unwrap().text(), retained);
    assert_eq!(p.compose.as_ref().unwrap().composer.input.cursor(), cursor);
    if is_create {
        assert!(matches!(&p.compose.as_ref().unwrap().draft,
            Draft::Description { project, kind, summary }
            if project == "ABC" && kind == &bug() && summary == "  SECRET_SUMMARY  "));
        if succeeds {
            assert_eq!(p.pages.last().unwrap().key, "ABC-31");
        }
    }
    assert!(host.plugins[0].capturing());
    let visible = crate::tests::shown(&host, 160, 40);
    assert!(
        visible.contains("SECRET_SECOND") && visible.contains("end"),
        "{visible}"
    );
    assert!(
        visible.contains(if is_create {
            "new Bug in ABC"
        } else {
            "comment on ABC-12"
        }),
        "{visible}"
    );

    // Completion never replays B. A plain y remains text until a fresh review.
    assert!(spawn_contexts(&key(&mut host, &mut all, KeyCode::Char('y'))).is_empty());
    assert!(host.menu.is_none());
    let edited = format!("{retained}y");
    assert_eq!(
        tests::jira_mut(&mut host).compose.as_ref().unwrap().text(),
        edited
    );
    review(&mut host, &mut all);
    assert!(host.menu.is_some());
    let fresh = tests::jira_mut(&mut host).change.clone().unwrap();
    assert_eq!(
        fresh,
        if is_create {
            create("acme", "ABC", &edited)
        } else {
            comment("acme", "ABC-12", &edited)
        }
    );
    let effects = key(&mut host, &mut all, KeyCode::Char('y'));
    assert_eq!(
        spawn_contexts(&effects).len(),
        1,
        "freshly reviewed text dispatches once"
    );
    assert!(tests::jira_mut(&mut host).compose.is_none());
    assert_private_history(all);
}

#[test]
fn host_busy_comment_keeps_editable_text_after_original_success() {
    busy_host_flow(false, true);
}

#[test]
fn host_busy_comment_keeps_editable_text_after_original_failure() {
    busy_host_flow(false, false);
}

#[test]
fn host_busy_create_keeps_summary_and_description_after_original_success() {
    busy_host_flow(true, true);
}

#[test]
fn host_busy_create_keeps_summary_and_description_after_original_failure() {
    busy_host_flow(true, false);
}
