//! Futures returned by real dispatch remain unpolled. Recorded callbacks drive
//! the reducer and Host; these tests never construct a live client or resolve a
//! token/email reference.

use super::*;
use crate::test_support::TicketBuilder;
use crate::{JiraMsg, Target, config, demo, detail_key, model, page, start, tests, write};
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::input::TextInput;
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::writes;
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

fn comment(key: &str) -> write::Change {
    write::Change::Comment {
        site: "acme".into(),
        key: key.into(),
        text: "SECRET_BODY".into(),
    }
}

fn moves() -> Vec<model::Transition> {
    vec![model::Transition {
        id: "21".into(),
        name: "Start".into(),
        to: "In Progress".into(),
        category: model::Category::Doing,
    }]
}

fn start_plan(key: &str) -> start::Start {
    // Keep the deliberately mismatched status/category from the legacy case:
    // the plan uses the category to decide whether a move is needed.
    let ticket = TicketBuilder::new()
        .key(key)
        .id("")
        .summary("Add a stats endpoint for the dashboard")
        .status("In Progress", model::Category::Todo)
        .assignee("SECRET_OLD_ACCOUNT", "Alice Example")
        .build();
    start::plan(
        "acme",
        &ticket,
        Some(&model::Me {
            account_id: "SECRET_ACCOUNT".into(),
            name: "SECRET_PERSON".into(),
        }),
        Some(&moves()),
        "",
    )
}

mod builder_parity {
    use super::*;

    #[test]
    fn minimal_ticket_preserves_legacy_start_dispatch_plan() {
        let original = demo::sample().data["acme"].tickets["ABC-12"].clone();
        for key in ["ABC-12", "ABC-13"] {
            let mut legacy = original.clone();
            legacy.key = key.into();
            legacy.category = model::Category::Todo;
            legacy.assignee_id = "SECRET_OLD_ACCOUNT".into();
            assert_eq!(
                start_plan(key),
                start::plan(
                    "acme",
                    &legacy,
                    Some(&model::Me {
                        account_id: "SECRET_ACCOUNT".into(),
                        name: "SECRET_PERSON".into(),
                    }),
                    Some(&moves()),
                    "",
                ),
            );
        }
    }
}

fn sent(p: &mut JiraPlugin, change: &write::Change) -> Dispatch {
    writes::set_mode(writes::Mode::Edit, None);
    let effects = write::send(p, change.clone());
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Spawn(_)))
            .count(),
        1
    );
    p.sent[&detail_key(change.site(), change.about())]
        .dispatch
        .clone()
        .unwrap()
}

fn configure(p: &mut JiraPlugin, cfg: &config::Settings) {
    p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
        .unwrap();
}

fn no_follow_up(effects: &[Effect]) {
    assert!(!effects.iter().any(|e| matches!(
        e,
        Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::Open(_) | Effect::Show(_)
    )));
}

#[test]
fn generations_track_only_configured_connection_and_site_incarnation() {
    let mut p = demo::sample();
    let dispatch = sent(&mut p, &comment("ABC-12"));
    let original = p.cfg.clone();
    let beta = p.connections.generation("beta");
    let mut unrelated = original.clone();
    unrelated.refresh_secs += 1;
    unrelated.idle_minutes += 1;
    unrelated.start_status = "Review".into();
    unrelated.on_start = config::OnStart::Off;
    unrelated.sites[0].color = "yellow".into();
    unrelated.sites[0].from = "SECRET_IMPORT_PATH".into();
    unrelated.sites[0].projects.clear();
    unrelated.views[0].name = "renamed view".into();
    unrelated.views[0].jql = "project = DEF".into();
    unrelated.style.clear();
    unrelated.sites.reverse();
    configure(&mut p, &unrelated);
    assert!(current(&mut p, "acme", &dispatch));
    assert_eq!(p.connections.generation("beta"), beta);

    for field in ["url", "email", "token", "name", "remove"] {
        let mut p = demo::sample();
        let dispatch = sent(&mut p, &comment("ABC-12"));
        let beta = p.connections.generation("beta");
        let mut changed = original.clone();
        match field {
            "url" => changed.sites[0].url = "https://replacement.example".into(),
            "email" => changed.sites[0].email = "env:SECRET_EMAIL_SOURCE".into(),
            "token" => changed.sites[0].token = "env:SECRET_TOKEN_SOURCE".into(),
            "name" => changed.sites[0].name = "renamed".into(),
            "remove" => {
                changed.sites.remove(0);
            }
            _ => unreachable!(),
        }
        configure(&mut p, &changed);
        assert!(!current(&mut p, "acme", &dispatch), "{field}");
        assert_eq!(
            p.connections.generation("beta"),
            beta,
            "other site unaffected"
        );
        configure(&mut p, &original);
        assert!(!current(&mut p, "acme", &dispatch), "A-B-A: {field}");
        assert!(!format!("{:?}", p.connections).contains("SECRET"));
    }
}

#[test]
fn editing_other_site_leaves_original_write_follow_ups_available() {
    let mut p = demo::sample();
    let change = comment("ABC-12");
    let dispatch = sent(&mut p, &change);
    let mut cfg = p.cfg.clone();
    cfg.sites[1].email = "SECRET_OTHER@example.com".into();
    configure(&mut p, &cfg);
    let effects = write::on_wrote(&mut p, change, Ok(Value::Null), &dispatch);
    assert!(effects.iter().any(|e| matches!(e, Effect::SpawnAction { .. })));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Observe { .. })));
}

#[test]
fn stale_failure_keeps_original_text_and_truth_without_follow_ups() {
    let mut p = demo::sample();
    let change = comment("ABC-12");
    let dispatch = sent(&mut p, &change);
    let mut cfg = p.cfg.clone();
    cfg.sites[0].token = "env:SECRET_CHANGED_SOURCE".into();
    configure(&mut p, &cfg);
    let effects = write::on_wrote(&mut p, change, Err("SECRET_RESPONSE".into()), &dispatch);
    assert!(matches!(effects[0], Effect::Report(Outcome::Unknown)));
    no_follow_up(&effects);
    assert_eq!(
        p.recovery.records.values().next().unwrap().original,
        crate::write_recovery::Original::Comment("SECRET_BODY".into())
    );
    assert!(
        p.sent[&detail_key("acme", "ABC-12")]
            .result
            .as_ref()
            .unwrap()
            .is_err()
    );
}

#[test]
fn duplicate_completion_never_reapplies_or_overwrites_a_finished_or_newer_write() {
    let mut p = demo::sample();
    let change = comment("ABC-12");
    let dispatch = sent(&mut p, &change);
    let _ = write::on_wrote(&mut p, change.clone(), Ok(Value::Null), &dispatch);
    let success = p.sent[&detail_key("acme", "ABC-12")].result.clone();
    for result in [Ok(Value::Null), Err("SECRET_DUPLICATE_ERROR".into())] {
        no_follow_up(&write::on_wrote(&mut p, change.clone(), result, &dispatch));
        assert_eq!(p.sent[&detail_key("acme", "ABC-12")].result, success);
        assert!(p.recovery.records.is_empty());
    }
    let new = sent(&mut p, &change);
    no_follow_up(&write::on_wrote(&mut p, change, Ok(Value::Null), &dispatch));
    let pending = &p.sent[&detail_key("acme", "ABC-12")];
    assert_eq!(pending.dispatch.as_ref().unwrap().id, new.id);
    assert!(pending.result.is_none());
}

#[test]
fn queued_move_belongs_to_exact_dispatch_and_preserves_another_pending_chain() {
    let mut p = demo::sample();
    writes::set_mode(writes::Mode::Edit, None);
    let plan = start_plan("ABC-12");
    let assign = plan.assign.clone().unwrap();
    p.starting = Some(plan.clone());
    let _ = start::on_menu(&mut p, "j-start", "yes").unwrap();
    let owner = p.start_next.as_ref().unwrap().owner.clone();
    // Busy same-ticket and independent other-ticket start requests cannot replace it.
    for key in ["ABC-12", "ABC-13"] {
        p.starting = Some(start_plan(key));
        let effects = start::on_menu(&mut p, "j-start", "yes").unwrap();
        no_follow_up(&effects);
        assert_eq!(p.start_next.as_ref().unwrap().owner.id, owner.id);
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::Report(Outcome::Blocked)))
                .count(),
            2
        );
    }
    let unrelated = comment("ABC-13");
    let other = sent(&mut p, &unrelated);
    assert!(start::after_wrote(&mut p, &unrelated, true, &other).is_empty());
    assert_eq!(p.start_next.as_ref().unwrap().owner.id, owner.id);
    let _ = write::on_wrote(&mut p, assign.clone(), Ok(Value::Null), &owner);
    let effects = start::after_wrote(&mut p, &assign, true, &owner);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Spawn(_)))
            .count(),
        1
    );
    let mv = plan.mv.clone().unwrap();
    let moved = p.sent[&detail_key("acme", "ABC-12")]
        .dispatch
        .clone()
        .unwrap();
    let _ = write::on_wrote(&mut p, mv, Ok(Value::Null), &moved);
    p.starting = Some(plan);
    let _ = start::on_menu(&mut p, "j-start", "yes").unwrap();
    let newer = p.start_next.as_ref().unwrap().owner.id;
    assert_ne!(newer, owner.id);
    no_follow_up(&write::on_wrote(
        &mut p,
        assign.clone(),
        Ok(Value::Null),
        &owner,
    ));
    assert!(start::after_wrote(&mut p, &assign, true, &owner).is_empty());
    assert_eq!(p.start_next.as_ref().unwrap().owner.id, newer);
}

#[test]
fn refused_first_step_never_queues_or_sends_move() {
    for refusal in ["busy", "mode", "missing", "failure"] {
        let mut p = demo::sample();
        let plan = start_plan("ABC-12");
        let assign = plan.assign.clone().unwrap();
        writes::set_mode(writes::Mode::Edit, None);
        match refusal {
            "busy" => {
                let _ = sent(&mut p, &comment("ABC-12"));
            }
            "mode" => writes::set_mode(writes::Mode::ReadOnly, None),
            "missing" => p.cfg.sites.clear(),
            _ => {}
        }
        p.starting = Some(plan);
        let effects = start::on_menu(&mut p, "j-start", "yes").unwrap();
        if refusal == "failure" {
            let owner = p.start_next.as_ref().unwrap().owner.clone();
            let _ = write::on_wrote(
                &mut p,
                assign.clone(),
                Err("SECRET_RESPONSE".into()),
                &owner,
            );
            let effects = start::after_wrote(&mut p, &assign, false, &owner);
            no_follow_up(&effects);
            assert!(matches!(
                effects.last(),
                Some(Effect::Report(Outcome::Blocked))
            ));
        } else {
            no_follow_up(&effects);
        }
        assert!(p.start_next.is_none(), "{refusal}");
    }
}

struct Run {
    host: Host,
    records: Vec<JournalRecord>,
}

struct Pending {
    task: ActionId,
    semantic: ActionId,
    dispatch: Dispatch,
    change: write::Change,
}

impl Run {
    fn new() -> Self {
        let mut host = tests::hosted();
        host.set_env_mode(Some((writes::Mode::Edit, "RADAR_MODE".into())));
        let p = tests::jira_mut(&mut host);
        let _ = page::open(p, "acme", "ABC-12");
        crate::tests::complete_detail(p, "acme", "ABC-12", Ok(tests::detail()));
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
    fn key(&mut self, code: KeyCode) -> Vec<HostEffect> {
        self.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }
    fn launch(&mut self, kind: &str) -> Pending {
        if kind == "start" {
            let t = &mut self
                .p()
                .details
                .get_mut(&detail_key("acme", "ABC-12"))
                .unwrap()
                .value
                .as_mut()
                .unwrap()
                .ticket;
            t.category = model::Category::Todo;
            t.assignee_id = "SECRET_OTHER_ACCOUNT".into();
            self.event(Event::Msg {
                plugin: 0,
                msg: Box::new(JiraMsg::Started {
                    site: "acme".into(),
                    key: "ABC-12".into(),
                    me: None,
                    moves: Some(Ok(moves())),
                }),
            });
            assert!(self.host.menu.is_some());
            // Preserve fixture-generated assignment, with real menu dispatch.
        } else {
            self.key(KeyCode::Char(if kind == "create" { 'n' } else { 'c' }));
            if kind == "create" {
                let read = self.p().dialogs.current_read();
                self.event(Event::Msg {
                    plugin: 0,
                    msg: Box::new(JiraMsg::Types {
                        read,
                        site: "acme".into(),
                        project: "ABC".into(),
                        result: Ok(vec![model::IssueType {
                            id: "1".into(),
                            name: "Task".into(),
                            subtask: false,
                        }]),
                    }),
                });
                self.key(KeyCode::Char('1'));
                self.event(Event::Paste("SECRET_SUMMARY".into()));
                self.key(KeyCode::Enter);
            }
            self.event(Event::Paste("SECRET_BODY".into()));
            self.event(Event::Key(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )));
        }
        let change = if kind == "start" {
            self.p().starting.as_ref().unwrap().assign.clone().unwrap()
        } else {
            self.p().change.clone().unwrap()
        };
        let effects = self.key(KeyCode::Char('y'));
        let contexts = spawns(&effects);
        assert_eq!(
            contexts.len(),
            1,
            "real write dispatched, task never polled"
        );
        let (task, semantic) = contexts.into_iter().next().unwrap();
        let dispatch = self.p().sent[&detail_key(change.site(), change.about())]
            .dispatch
            .clone()
            .unwrap();
        Pending {
            task,
            semantic,
            dispatch,
            change,
        }
    }
    fn edit(&mut self, target: Target, value: &str) {
        self.host.view = View::Settings;
        self.host.section = self
            .host
            .sections
            .iter()
            .position(|(_, s)| s.group == "jira" && s.id == "connection")
            .unwrap();
        self.host.focus = Pane::Middle;
        self.p().edit = Some((target, TextInput::new(value)));
        self.key(KeyCode::Enter);
        assert!(self.p().edit.is_none(), "host committed configured field");
        if self.host.menu.is_some() {
            self.key(KeyCode::Esc);
        }
    }
    fn complete(&mut self, pending: &Pending, succeeds: bool) -> Vec<HostEffect> {
        self.event(Event::Tracked {
            action: pending.task.clone(),
            semantic: Some(pending.semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(JiraMsg::Wrote {
                    dispatch: pending.dispatch.clone(),
                    change: pending.change.clone(),
                    result: if succeeds {
                        Ok(json!({"key":"ABC-31", "SECRET_RESPONSE":"never journal"}))
                    } else {
                        Err("SECRET_RESPONSE".into())
                    },
                }),
            }),
        })
    }
    fn private(self) {
        let dir =
            std::env::temp_dir().join(format!("radar-jira-context-{}", ActionId::new().as_str()));
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
        let text = serde_json::to_string(&events).unwrap();
        for secret in [
            "SECRET",
            "alice@example.com",
            "RADAR_JIRA_TEST_TOKEN",
            "email",
            "token",
        ] {
            assert!(!text.contains(secret), "journal must not contain {secret}");
        }
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
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

#[test]
fn host_connection_edit_fences_comment_create_and_chained_move_but_keeps_actual_outcomes() {
    for kind in ["comment", "create", "start"] {
        for succeeds in [true, false] {
            let mut r = Run::new();
            let pending = r.launch(kind);
            let pages: Vec<_> = r
                .p()
                .pages
                .iter()
                .map(|p| (p.site.clone(), p.key.clone()))
                .collect();
            r.edit(Target::SiteUrl(0), "https://replacement.example");
            let first = r.records.len();
            let effects = r.complete(&pending, succeeds);
            assert!(spawns(&effects).is_empty());
            assert_eq!(
                r.p()
                    .pages
                    .iter()
                    .map(|p| (p.site.clone(), p.key.clone()))
                    .collect::<Vec<_>>(),
                pages
            );
            let records = &r.records[first..];
            let actual: Vec<_> = records
                .iter()
                .filter_map(|r| match r {
                    JournalRecord::Outcome { id, outcome, .. } if id == &pending.semantic => {
                        Some(*outcome)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                actual,
                [if succeeds {
                    Outcome::Succeeded
                } else {
                    Outcome::Unknown
                }]
            );
            for record in records {
                if let JournalRecord::Begin { descriptor, .. } = record {
                    for subject in &descriptor.subjects {
                        assert_eq!(subject.key.instance, "https://acme.atlassian.net/");
                    }
                }
            }
            if succeeds {
                assert!(records.iter().any(|r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.write.follow_up.skipped" && descriptor.metadata.count == Some(1))));
                if kind == "create" {
                    assert!(records.iter().any(|r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.ticket.create.result" && descriptor.subjects[0].key.id == "ABC-31")));
                }
            }
            if kind == "start" {
                let blocked = records
                    .iter()
                    .find_map(|r| match r {
                        JournalRecord::Begin { id, descriptor, .. }
                            if descriptor.kind.as_str() == "jira.ticket.move" =>
                        {
                            Some(id)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(records.iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Blocked, .. } if id == blocked)));
                assert!(r.p().start_next.is_none());
            }
            assert!(
                r.p().sent[&detail_key(pending.change.site(), pending.change.about())]
                    .result
                    .is_some()
            );
            if !succeeds && kind != "start" {
                assert_eq!(
                    r.p().recovery.records.values().next().unwrap().original,
                    if kind == "create" {
                        crate::write_recovery::Original::Create {
                            summary: "SECRET_SUMMARY".into(),
                            description: "SECRET_BODY".into(),
                        }
                    } else {
                        crate::write_recovery::Original::Comment("SECRET_BODY".into())
                    }
                );
            }
            let notice = r.host.message.as_deref().unwrap();
            assert!(
                notice.contains(if succeeds {
                    if kind == "start" {
                        "Assignment succeeded"
                    } else if kind == "create" {
                        "created ABC-31"
                    } else {
                        "comment posted"
                    }
                } else {
                    "SECRET_RESPONSE"
                }),
                "{notice}"
            );
            r.private();
        }
    }
}

#[test]
fn host_connection_mutation_routes_keep_old_dispatch_stale_even_after_return_to_original() {
    for route in ["url", "email", "token", "name", "remove"] {
        let mut r = Run::new();
        let pending = r.launch("start");
        let original = r.p().cfg.sites[0].clone();
        match route {
            "url" => {
                r.edit(Target::SiteUrl(0), "https://replacement.example");
                r.edit(Target::SiteUrl(0), &original.url);
            }
            "email" => {
                r.edit(Target::SiteEmail(0), "SECRET_OTHER@example.com");
                r.edit(Target::SiteEmail(0), &original.email);
            }
            "token" => {
                r.edit(Target::SiteToken(0), "SECRET_FAKE_TOKEN_123456");
                let mut cfg = r.p().cfg.clone();
                cfg.sites[0].token = original.token.clone();
                configure(r.p(), &cfg);
            }
            "name" => {
                r.edit(Target::SiteName(0), "renamed");
                r.edit(Target::SiteName(0), "acme");
            }
            "remove" => {
                let _ = r.p().on_menu("site-remove:0", "yes");
                assert!(r.p().cfg.site("acme").is_none());
                r.host.view = View::Settings;
                r.host.section = r
                    .host
                    .sections
                    .iter()
                    .position(|(_, s)| s.group == "jira" && s.id == "connection")
                    .unwrap();
                r.host.focus = Pane::Middle;
                r.p().edit = Some((Target::SiteAdd, TextInput::new(&original.url)));
                r.key(KeyCode::Enter);
                assert!(r.p().cfg.site("acme").is_some());
                r.key(KeyCode::Esc); // finish the add flow's next-field editor
                let mut cfg = r.p().cfg.clone();
                *cfg.sites.iter_mut().find(|s| s.name == "acme").unwrap() = original;
                configure(r.p(), &cfg);
            }
            _ => unreachable!(),
        }
        assert!(!current(r.p(), "acme", &pending.dispatch), "{route}");
        assert!(spawns(&r.complete(&pending, true)).is_empty(), "{route}");
        assert!(r.p().start_next.is_none());
        r.private();
    }
}

#[test]
fn host_unchanged_start_completes_in_order_and_rechecks_mode_before_move() {
    for refuse_move in [false, true] {
        let mut r = Run::new();
        let assign = r.launch("start");
        let mv = r.p().start_next.as_ref().unwrap().change.clone();
        if refuse_move {
            r.host
                .set_env_mode(Some((writes::Mode::ReadOnly, "RADAR_MODE".into())));
        }
        let first = r.records.len();
        let effects = r.complete(&assign, true);
        assert!(r.p().start_next.is_none());
        let move_action = r.records[first..]
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "jira.ticket.move" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        assert!(r.records[first..].iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Succeeded, .. } if id == &assign.semantic)));
        let move_tasks: Vec<_> = spawns(&effects)
            .into_iter()
            .filter(|(_, semantic)| semantic == &move_action)
            .collect();
        if refuse_move {
            assert!(move_tasks.is_empty());
            assert!(r.records[first..].iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Blocked, .. } if id == &move_action)));
        } else {
            assert_eq!(move_tasks.len(), 1);
            let (task, semantic) = move_tasks.into_iter().next().unwrap();
            let dispatch = r.p().sent[&detail_key("acme", "ABC-12")]
                .dispatch
                .clone()
                .unwrap();
            assert_ne!(dispatch.id, assign.dispatch.id);
            let pending = Pending {
                task,
                semantic,
                dispatch,
                change: mv,
            };
            let _ = r.complete(&pending, true);
            assert!(r.records.iter().any(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Succeeded, .. } if id == &move_action)));
        }
        r.private();
    }
}
