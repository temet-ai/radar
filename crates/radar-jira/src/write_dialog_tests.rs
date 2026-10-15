//! Real Host regressions for late action reads. Every spawned future is dropped
//! unpolled: fixtures supply replies, and no HTTP or authentication is performed.

use super::*;
use crate::{tests, write_dialog::Read};
use radar_core::Host;
use radar_core::Plugin;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};

const BODY: &str = "  SECRET_BODY\n\nlast  ";

#[derive(Clone, Copy, Debug)]
enum Kind {
    Types,
    Moves,
    Who,
}

impl Kind {
    fn key(self) -> char {
        match self {
            Self::Types => 'n',
            Self::Moves => 't',
            Self::Who => 'i',
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Types => "jira.read.types",
            Self::Moves => "jira.read.moves",
            Self::Who => "jira.read.who",
        }
    }
}

#[derive(Clone)]
struct Pending {
    read: Read,
    task: ActionId,
    semantic: ActionId,
    kind: Kind,
    target: String,
}

struct Run {
    host: Host,
    records: Vec<JournalRecord>,
}

fn bug() -> IssueType {
    IssueType {
        id: "10004".into(),
        name: "Bug".into(),
        subtask: false,
    }
}
fn moves() -> Vec<Transition> {
    vec![Transition {
        id: "21".into(),
        name: "Review".into(),
        to: "In Review".into(),
        category: Category::Doing,
    }]
}
fn who() -> Me {
    Me {
        account_id: "SECRET_ACCOUNT".into(),
        name: "SECRET_NAME".into(),
    }
}

fn spawns(effects: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    effects
        .iter()
        .filter_map(|e| match e {
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

impl Run {
    fn new(page: bool) -> Self {
        let mut host = tests::hosted();
        host.set_env_mode(Some((writes::Mode::Write, "RADAR_MODE".into())));
        if page {
            page::open(tests::jira_mut(&mut host), "acme", "ABC-12");
            crate::tests::complete_detail(
                tests::jira_mut(&mut host),
                "acme",
                "ABC-12",
                Ok(tests::detail()),
            );
        }
        host.enable_journal();
        Self {
            host,
            records: vec![],
        }
    }
    fn p(&mut self) -> &mut JiraPlugin {
        tests::jira_mut(&mut self.host)
    }
    fn event(&mut self, event: Event) -> Vec<HostEffect> {
        let effects = self.host.update(event);
        self.records.extend(effects.iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        }));
        effects
    }
    fn key(&mut self, key: KeyCode) -> Vec<HostEffect> {
        self.event(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)))
    }
    fn char(&mut self, key: char) -> Vec<HostEffect> {
        self.key(KeyCode::Char(key))
    }
    fn paste(&mut self, text: &str) {
        self.event(Event::Paste(text.into()));
    }
    fn review(&mut self) {
        self.event(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
    }
    fn capture(&mut self, kind: Kind, target: &str, effects: Vec<HostEffect>) -> Pending {
        let contexts = spawns(&effects);
        assert_eq!(
            contexts.len(),
            1,
            "one real action read was dispatched, never polled"
        );
        let (task, semantic) = contexts[0].clone();
        let read = self.p().dialogs.current_read();
        assert_eq!(read.action.kind.as_str(), kind.operation());
        assert_eq!(read.action.subjects[0].key.id, target);
        Pending {
            read,
            task,
            semantic,
            kind,
            target: target.into(),
        }
    }
    fn launch(&mut self, kind: Kind) -> Pending {
        if matches!(kind, Kind::Who) {
            self.p().data.get_mut("acme").unwrap().me = None;
        }
        let effects = self.char(kind.key());
        self.capture(
            kind,
            if matches!(kind, Kind::Types) {
                "ABC"
            } else {
                "ABC-12"
            },
            effects,
        )
    }
    fn project(&mut self, key: char, project: &str) -> Pending {
        self.char('n');
        assert_eq!(
            self.host.menu.as_ref().unwrap().menu.title,
            "a new ticket in which project?"
        );
        let effects = self.char(key);
        self.capture(Kind::Types, project, effects)
    }
    fn reply(&mut self, pending: &Pending, failed: bool) -> Vec<HostEffect> {
        let read = pending.read.clone();
        let result = |ok| {
            if failed {
                Err("SECRET_PROVIDER_ERROR".into())
            } else {
                Ok(ok)
            }
        };
        let msg = match pending.kind {
            Kind::Types => JiraMsg::Types {
                read,
                site: "acme".into(),
                project: pending.target.clone(),
                result: result(vec![bug()]),
            },
            Kind::Moves => JiraMsg::Moves {
                read,
                site: "acme".into(),
                key: pending.target.clone(),
                result: if failed {
                    Err("SECRET_PROVIDER_ERROR".into())
                } else {
                    Ok(moves())
                },
            },
            Kind::Who => JiraMsg::Who {
                read,
                site: "acme".into(),
                key: pending.target.clone(),
                result: if failed {
                    Err("SECRET_PROVIDER_ERROR".into())
                } else {
                    Ok(who())
                },
            },
        };
        self.event(Event::Tracked {
            action: pending.task.clone(),
            semantic: Some(pending.semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(msg),
            }),
        })
    }
    fn editor(&mut self) -> (String, Draft, String, usize) {
        let c = self.p().compose.as_ref().unwrap();
        (
            c.site.clone(),
            c.draft.clone(),
            c.text(),
            c.composer.input.cursor(),
        )
    }
    fn menu(&self) -> Option<radar_core::plugin::Menu> {
        self.host.menu.as_ref().map(|m| m.menu.clone())
    }
    fn outcomes(&self, id: &ActionId) -> Vec<JournalOutcome> {
        self.records
            .iter()
            .filter_map(|r| match r {
                JournalRecord::Outcome {
                    id: got, outcome, ..
                } if got == id => Some(*outcome),
                _ => None,
            })
            .collect()
    }
    fn assert_skipped(&self, pending: &Pending) {
        assert_eq!(
            self.outcomes(&pending.semantic),
            [JournalOutcome::Succeeded]
        );
        assert!(
            !self
                .outcomes(&pending.semantic)
                .contains(&JournalOutcome::Blocked)
        );
        assert!(self.records.iter().any(|r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.action_menu.application.skipped" && descriptor.metadata.count == Some(1))));
        assert!(
            self.host
                .message
                .as_deref()
                .is_some_and(|s| s.contains("superseded")
                    && s.contains(&format!("press {}", pending.kind.key())))
        );
    }
    fn assert_private(self) {
        let dir = std::env::temp_dir().join(format!("radar-jira-dialog-{}", ActionId::new().as_str()));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in self.records {
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
                JournalRecord::Gap { .. } => panic!("history gap"),
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
}

#[test]
fn late_types_moves_and_who_leave_comment_and_its_review_exactly_owned() {
    for kind in [Kind::Types, Kind::Moves, Kind::Who] {
        for reviewing in [false, true] {
            let mut r = Run::new(true);
            let pending = r.launch(kind);
            r.char('c');
            r.paste(BODY);
            r.key(KeyCode::Left);
            if reviewing {
                r.review();
            }
            let editor = r.editor();
            let change = r.p().change.clone();
            let menu = r.menu();
            let effects = r.reply(&pending, false);
            assert!(spawns(&effects).is_empty());
            assert_eq!(r.editor(), editor, "{kind:?}");
            assert_eq!(r.p().change, change);
            assert_eq!(r.menu(), menu);
            assert!(r.p().moving.is_none() && r.p().new_ticket.is_none());
            r.assert_skipped(&pending);
            if reviewing {
                // No async action menu appeared: n declines this exact review.
                r.char('e');
                assert_eq!(r.editor(), editor);
            }
            r.char('q');
            r.paste(" SECRET_PASTE\nend  ");
            assert!(!r.host.quit);
            assert!(r.host.plugins[0].capturing());
            assert!(r.editor().2.contains("q SECRET_PASTE"));
            let visible = radar_core::shell::to_text(&r.host, 160, 40);
            assert!(visible.contains("SECRET_BODY") && visible.contains("comment on ABC-12"));
            r.assert_private();
        }
    }
}

#[test]
fn late_action_reads_leave_create_summary_description_and_review_exact() {
    for kind in [Kind::Types, Kind::Moves, Kind::Who] {
        for stage in 0..3 {
            let mut r = Run::new(!matches!(kind, Kind::Types));
            let pending = if matches!(kind, Kind::Types) {
                r.project('2', "DEF")
            } else {
                r.launch(kind)
            };
            let create = if matches!(kind, Kind::Types) {
                r.project('1', "ABC")
            } else {
                r.launch(Kind::Types)
            };
            r.reply(&create, false);
            r.char('1');
            r.paste("  SECRET_SUMMARY  ");
            if stage > 0 {
                r.key(KeyCode::Enter);
                r.paste(BODY);
            }
            r.key(KeyCode::Left);
            if stage == 2 {
                r.review();
            }
            let editor = r.editor();
            let change = r.p().change.clone();
            let menu = r.menu();
            assert!(spawns(&r.reply(&pending, false)).is_empty());
            assert_eq!(r.editor(), editor, "{kind:?}, stage {stage}");
            assert_eq!(r.p().change, change);
            assert_eq!(r.menu(), menu);
            r.assert_skipped(&pending);
            if stage == 2 {
                r.char('e');
            }
            r.char('q');
            r.paste(" SECRET_PASTE");
            assert!(!r.host.quit && r.host.plugins[0].capturing());
            assert!(r.editor().2.contains("SECRET_PASTE"));
            assert!(r.p().sent.is_empty());
            r.assert_private();
        }
    }
}

#[test]
fn unchanged_idle_action_reads_still_open_their_normal_single_use_prompts() {
    for kind in [Kind::Types, Kind::Moves, Kind::Who] {
        let mut r = Run::new(true);
        let pending = r.launch(kind);
        r.reply(&pending, false);
        assert!(
            r.menu().is_some(),
            "{kind:?}: launch input must not stale its own read"
        );
        match kind {
            Kind::Types => {
                r.char('1');
                assert!(matches!(r.editor().1, Draft::Summary { .. }));
            }
            Kind::Moves => {
                r.char('1');
                assert!(matches!(r.p().change, Some(Change::Move { .. })));
                r.char('n');
            }
            Kind::Who => {
                assert!(matches!(r.p().change, Some(Change::Assign { .. })));
                r.char('n');
            }
        }
        assert!(r.p().sent.is_empty());
        r.assert_private();
    }
}

#[test]
fn repeated_requests_and_reverse_order_replies_never_revive_old_dialogs() {
    let mut r = Run::new(false);
    let old = r.project('1', "ABC");
    let new = r.project('2', "DEF");
    r.reply(&new, false);
    let shown = r.menu();
    r.reply(&old, false);
    assert_eq!(r.menu(), shown);
    r.assert_skipped(&old);
    r.char('1');
    assert!(matches!(r.editor().1, Draft::Summary { project, .. } if project == "DEF"));
    r.key(KeyCode::Esc);
    // A repeated launch is deduplicated, and explicit later input supersedes it.
    let next = r.project('1', "ABC");
    r.char('n');
    let duplicate = r.char('1');
    assert!(spawns(&duplicate).is_empty());
    r.reply(&next, false);
    assert!(r.menu().is_none());
    r.assert_skipped(&next);
    let retry = r.project('1', "ABC");
    r.reply(&retry, false);
    assert!(r.menu().is_some());
}

#[test]
fn escape_cancels_pending_read_and_does_not_lock_out_a_new_action_after_a_menu() {
    for kind in [Kind::Types, Kind::Moves, Kind::Who] {
        let mut r = Run::new(true);
        let pending = r.launch(kind);
        r.key(KeyCode::Esc);
        r.reply(&pending, false);
        assert!(r.menu().is_none());
        r.assert_skipped(&pending);
        // Restore the fixture page after normal Host Escape navigation.
        page::open(r.p(), "acme", "ABC-12");
        let fresh = r.launch(kind);
        r.reply(&fresh, false);
        if matches!(kind, Kind::Moves) {
            r.char('1');
        }
        let old_menu = r.menu().unwrap().id;
        r.key(KeyCode::Esc);
        r.char('c');
        r.paste(BODY);
        let editor = r.editor();
        // A stale callback is also refused defensively after a new editor owns input.
        let effects = r.p().on_menu(&old_menu, "no");
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::Observe {
                outcome: JournalOutcome::Blocked,
                ..
            }
        )));
        assert_eq!(r.editor(), editor);
        assert!(r.p().change.is_none());
        assert!(r.p().sent.is_empty());
    }
}

#[test]
fn later_host_menu_paste_or_mouse_input_supersedes_the_read_without_hidden_capture() {
    for later in 0..3 {
        let mut r = Run::new(true);
        let pending = r.launch(Kind::Types);
        match later {
            0 => {
                r.char('x');
            }
            1 => r.paste("SECRET_IGNORED_PASTE"),
            _ => {
                r.event(Event::Mouse {
                    kind: radar_core::host::MouseKind::Press,
                    column: 2,
                    row: 2,
                    at_ms: 100,
                });
            }
        }
        let menu = r.menu();
        r.reply(&pending, false);
        assert_eq!(r.menu(), menu);
        assert!(!r.host.plugins[0].capturing());
        assert!(r.p().compose.is_none() && r.p().new_ticket.is_none());
        r.assert_skipped(&pending);
    }
}

#[test]
fn a_late_read_preserves_a_review_and_reports_failure_only_to_original_read() {
    let mut r = Run::new(true);
    let pending = r.launch(Kind::Types);
    r.char('c');
    r.paste(BODY);
    r.review();
    let shown = r.menu();
    let editor = r.editor();
    let change = r.p().change.clone();
    let reviewed = r
        .records
        .iter()
        .find_map(|rec| match rec {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.comment.post" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    r.reply(&pending, true);
    assert_eq!(r.outcomes(&pending.semantic), [JournalOutcome::Failed]);
    assert!(r.outcomes(&reviewed).is_empty());
    assert_eq!(r.menu(), shown);
    assert_eq!(r.editor(), editor);
    assert_eq!(r.p().change, change);
    r.char('n');
    assert_eq!(r.outcomes(&reviewed), [JournalOutcome::Cancelled]);
    assert!(r.p().compose.is_none());
    assert_eq!(
        r.p().recovery.records.values().next().unwrap().original,
        crate::write_recovery::Original::Comment(BODY.into())
    );
    r.assert_private();
}

#[test]
fn background_start_suggestion_keeps_editor_and_existing_start_review_still_runs_in_order() {
    let mut r = Run::new(true);
    r.char('c');
    r.paste(BODY);
    r.review();
    let menu = r.menu();
    let editor = r.editor();
    let change = r.p().change.clone();
    r.event(Event::Msg {
        plugin: 0,
        msg: Box::new(JiraMsg::Started {
            site: "acme".into(),
            key: "ABC-12".into(),
            me: Some(Ok(who())),
            moves: Some(Ok(moves())),
        }),
    });
    assert_eq!(r.menu(), menu);
    assert_eq!(r.editor(), editor);
    assert_eq!(r.p().change, change);
    assert!(r.p().starting.is_none() && r.p().sent.is_empty());

    let mut r = Run::new(true);
    let pending = r.launch(Kind::Types);
    r.event(Event::Msg {
        plugin: 0,
        msg: Box::new(JiraMsg::Started {
            site: "acme".into(),
            key: "ABC-12".into(),
            me: Some(Ok(who())),
            moves: Some(Ok(moves())),
        }),
    });
    let menu = r.menu();
    let start = r.p().starting.clone();
    assert!(start.is_some());
    r.reply(&pending, false);
    assert_eq!(r.menu(), menu);
    assert_eq!(r.p().starting, start);
    assert!(spawns(&r.char('n')).is_empty());
    assert!(r.p().starting.is_none());
}

struct Contributor;

impl Plugin for Contributor {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<radar_core::plugin::ScreenSpec> {
        vec![radar_core::plugin::ScreenSpec {
            id: "fixture",
            name: "fixture",
            key: 'Z',
        }]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<radar_core::Subject> {
        Some(
            radar_core::Subject::new("merge-request", "fixture!1", "ABC-12: example")
                .with("title", "ABC-12: example"),
        )
    }
}

#[test]
fn contributed_host_action_consumes_only_its_originating_input_and_later_global_input_cancels() {
    for cancel in [false, true] {
        let mut r = Run::new(false);
        r.host = Host::new(
            vec![Box::new(crate::demo::sample()), Box::new(Contributor)],
            0,
        )
        .unwrap();
        r.host.load_config(Some("")).unwrap();
        r.host.plugins[0] = Box::new(crate::demo::sample());
        r.host.enable_journal();
        r.event(Event::Key(KeyEvent::new(
            KeyCode::Char('Z'),
            KeyModifiers::SHIFT,
        )));
        // Host resolves Jira's contributed move key from the fixture subject.
        let effects = r.char('>');
        let pending = r.capture(Kind::Moves, "ABC-12", effects);
        if cancel {
            r.char('x');
        }
        let menu = r.menu();
        r.reply(&pending, false);
        if cancel {
            assert_eq!(r.menu(), menu);
            r.assert_skipped(&pending);
        } else {
            assert_eq!(r.menu().unwrap().title, "move ABC-12 to…");
        }
    }
}

#[test]
fn stale_reply_does_not_release_a_new_same_target_request_or_reapply_its_menu() {
    let mut r = Run::new(true);
    let old = r.launch(Kind::Types);
    r.char('x');
    r.key(KeyCode::Esc);
    r.reply(&old, false);
    let new = r.launch(Kind::Types);
    let asking = r.p().asking.clone();
    r.reply(&old, false);
    assert_eq!(
        r.p().asking,
        asking,
        "duplicate old result cannot finish a newer request"
    );
    assert!(r.menu().is_none());
    r.reply(&new, false);
    let menu = r.menu().unwrap();
    r.char('1');
    r.paste("SECRET_SUMMARY");
    let editor = r.editor();
    r.reply(&new, false);
    assert!(r.menu().is_none());
    assert_eq!(r.editor(), editor);
    let effects = r.p().on_menu(&menu.id, "10004");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
    );
    assert_eq!(r.editor(), editor);
}

#[test]
fn read_application_skip_keeps_captured_subject_after_configuration_changes_without_logging_payload()
 {
    let mut r = Run::new(true);
    r.p().cfg.sites[0].url =
        "https://SECRET_USER:SECRET_TOKEN@original.example/path?token=SECRET_QUERY".into();
    let pending = r.launch(Kind::Types);
    r.char('c');
    r.paste(BODY);
    r.review();
    r.p().cfg.sites[0].url = "https://replacement.example".into();
    r.reply(&pending, false);
    let skipped = r
        .records
        .iter()
        .find_map(|rec| match rec {
            JournalRecord::Begin { descriptor, .. }
                if descriptor.kind.as_str() == "jira.action_menu.application.skipped" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        skipped.subjects[0].key.instance,
        "https://original.example/"
    );
    assert_eq!(skipped.subjects[0].key.id, "ABC");
    r.assert_private();
}

#[test]
fn stale_errors_and_empty_parsed_results_keep_actual_outcome_notice_and_skip_evidence() {
    for kind in [Kind::Types, Kind::Moves, Kind::Who] {
        for failed in [false, true] {
            let mut r = Run::new(true);
            let pending = r.launch(kind);
            r.char('c');
            r.paste(BODY);
            r.review();
            let editor = r.editor();
            let menu = r.menu();
            let change = r.p().change.clone();
            if failed {
                r.reply(&pending, true);
            } else {
                let read = pending.read.clone();
                let msg = match kind {
                    Kind::Types => JiraMsg::Types {
                        read,
                        site: "acme".into(),
                        project: "ABC".into(),
                        result: Ok(vec![]),
                    },
                    Kind::Moves => JiraMsg::Moves {
                        read,
                        site: "acme".into(),
                        key: "ABC-12".into(),
                        result: Ok(vec![]),
                    },
                    Kind::Who => JiraMsg::Who {
                        read,
                        site: "acme".into(),
                        key: "ABC-12".into(),
                        result: Ok(Me {
                            account_id: String::new(),
                            name: String::new(),
                        }),
                    },
                };
                r.event(Event::Tracked {
                    action: pending.task.clone(),
                    semantic: Some(pending.semantic.clone()),
                    event: Box::new(Event::Msg {
                        plugin: 0,
                        msg: Box::new(msg),
                    }),
                });
            }
            assert_eq!(
                r.outcomes(&pending.semantic),
                [if failed {
                    JournalOutcome::Failed
                } else {
                    JournalOutcome::Succeeded
                }]
            );
            assert_eq!(r.editor(), editor);
            assert_eq!(r.menu(), menu);
            assert_eq!(r.p().change, change);
            let notice = r.host.message.as_deref().unwrap();
            assert!(
                notice.contains("superseded") && notice.contains(&format!("press {}", kind.key()))
            );
            assert!(notice.contains(if failed {
                "could not ask"
            } else {
                match kind {
                    Kind::Types => "takes no tickets",
                    Kind::Moves => "has no moves",
                    Kind::Who => "did not say who",
                }
            }));
            assert!(r.records.iter().any(|rec| matches!(rec, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.action_menu.application.skipped" && descriptor.metadata.count == Some(1))));
            r.assert_private();
        }
    }
}
