use super::*;
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionEvent, ActionId, Journal, Query};
use radar_core::plugin::Task;

enum Begin {
    Start,
    Reconfigure,
}

struct Harness {
    plugin: Arc<Mutex<JiraPlugin>>,
    reads: Vec<(Site, Arc<Routed>)>,
}

impl Plugin for Harness {
    fn id(&self) -> &'static str {
        "jira"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }

    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        match msg.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    let mut fx = vec![Effect::Journal(ActionDescriptor::new(
                        "jira.test.write",
                        Origin::Plugin("jira".into()),
                    ))];
                    for (site, transport) in self.reads.drain(..) {
                        let owner = {
                            let mut plugin = self.plugin.lock().unwrap();
                            let mut configured = site.clone();
                            configured.name = "acme".into();
                            plugin.cfg.sites = vec![configured];
                            crate::tests::detail_read(&mut plugin, "acme", "ABC-12")
                        };
                        fx.push(spawn(
                            action(&site, "ABC-12"),
                            "acme".into(),
                            "ABC-12".into(),
                            owner,
                            async move {
                                Client::over(transport)
                                    .detail_observed("ABC-12", None)
                                    .await
                                    .map_err(|e| format!("{e:#}"))
                            },
                        ));
                    }
                    fx.push(Effect::Notify("SECRET_NOTICE".into()));
                    fx.push(Effect::Report(Outcome::Succeeded));
                    fx
                }
                Begin::Reconfigure => {
                    let mut plugin = self.plugin.lock().unwrap();
                    plugin.cfg.sites.clear();
                    crate::write_context::reconcile(&mut plugin);
                    plugin.cfg.views.clear();
                    plugin.pages.clear();
                    vec![]
                }
            },
            Err(msg) => self.plugin.lock().unwrap().on_msg(msg),
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

fn tasks(effects: Vec<HostEffect>) -> Vec<(ActionId, ActionId, Task)> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
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
        .collect()
}

fn export(records: Vec<JournalRecord>, finish: bool) -> Vec<ActionEvent> {
    let dir = std::env::temp_dir().join(format!(
        "radar-jira-detail-history-{}",
        ActionId::new().as_str()
    ));
    let mut db = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = db.start_session().unwrap();
    for record in records {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => db
                .begin_with_id(&session, &id, descriptor, parent.as_ref())
                .unwrap(),
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                db.record(&id, outcome, metadata).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected journal gap"),
        }
    }
    if finish {
        db.finish_session(&session).unwrap();
    }
    let events = db
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let exported = serde_json::to_string(&events).unwrap();
    assert!(!exported.contains("SECRET"), "{exported}");
    for private in [
        "elsewhere.example",
        "attachment.example",
        "parent =",
        "customfield",
        "10012",
    ] {
        assert!(
            !exported.contains(private),
            "{private} must not enter history"
        );
    }
    for entry in std::fs::read_dir(&dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(b"SECRET".len())
                .any(|window| window == b"SECRET"),
            "raw private text must not reach SQLite or its sidecars"
        );
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
    events
}

fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn open_begin(
    records: &[JournalRecord],
    kind: &str,
) -> (ActionId, ActionDescriptor, Option<ActionId>) {
    let found: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } if descriptor.kind.as_str() == kind => {
                Some((id.clone(), descriptor.clone(), parent.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one {kind} action");
    found.into_iter().next().unwrap()
}

fn selected_host(cached: bool) -> Host {
    let mut host = crate::tests::hosted();
    let p = crate::tests::jira_mut(&mut host);
    if cached {
        let mut detail = crate::tests::detail();
        detail.ticket.summary = "SECRET_TITLE".into();
        detail.ticket.description = demo::doc("SECRET_BODY");
        crate::tests::complete_detail(p, "acme", "ABC-12", Ok(detail));
    }
    p.tree.borrow_mut().select(vec![
        crate::screen::site_id("acme"),
        "g:acme:In Progress".into(),
        crate::screen::ticket_id("acme", "ABC-1"),
        crate::screen::ticket_id("acme", "ABC-12"),
    ]);
    host.enable_journal();
    host
}

#[test]
fn cached_direct_enter_records_local_open_without_read_or_raw_text_in_sqlite() {
    let mut host = selected_host(true);
    let before = crate::tests::jira_mut(&mut host).snapshot();
    let mut all = records(&press(&mut host, KeyCode::Char(':')));
    all.extend(records(&host.update(Event::Paste("SECRET_INPUT".into()))));
    all.extend(records(&press(&mut host, KeyCode::Esc)));
    let entered = press(&mut host, KeyCode::Enter);
    let got = records(&entered);
    let (opened, scope, parent) = open_begin(&got, "jira.page.open");
    let (input, _, input_parent) = open_begin(&got, "plugin.input");
    let (event, _, _) = open_begin(&got, "ui.action");
    assert_eq!(parent, Some(input));
    assert_eq!(input_parent, Some(event));
    assert_eq!(terminal(&got, &opened), [Outcome::Succeeded]);
    assert_eq!(
        scope.subjects[0].key.instance,
        "https://acme.atlassian.net/"
    );
    assert_eq!(scope.subjects[0].key.id, "ABC-12");
    assert!(tasks(entered).is_empty());
    assert_eq!(host.focus, radar_core::Pane::Middle);
    assert_eq!(crate::tests::jira_mut(&mut host).pages.len(), 1);
    all.extend(got);
    all.extend(records(&press(&mut host, KeyCode::Esc)));
    let reopened = press(&mut host, KeyCode::Enter);
    let got = records(&reopened);
    let (again, _, _) = open_begin(&got, "jira.page.open");
    assert_ne!(again, opened);
    assert!(tasks(reopened).is_empty());
    assert_eq!(crate::tests::jira_mut(&mut host).snapshot(), before);
    all.extend(got);
    let events = export(all, true);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind.as_str() == "jira.page.open"
                && event.outcome == Outcome::Succeeded)
            .count(),
        2
    );
    assert!(
        !events
            .iter()
            .any(|event| event.kind.as_str() == "jira.page.detail")
    );
}

#[test]
fn cold_direct_enter_and_inflight_reopen_stay_successful_after_detail_failure() {
    let mut host = selected_host(false);
    let entered = press(&mut host, KeyCode::Enter);
    let mut all = records(&entered);
    let (opened, _, open_parent) = open_begin(&all, "jira.page.open");
    let (detail, scope, detail_parent) = open_begin(&all, "jira.page.detail");
    assert_eq!(
        open_parent, detail_parent,
        "read and local open are siblings"
    );
    assert_ne!(open_parent, Some(opened.clone()));
    assert_eq!(terminal(&all, &opened), [Outcome::Succeeded]);
    assert!(terminal(&all, &detail).is_empty());
    let mut pending = tasks(entered);
    assert_eq!(pending.len(), 1);
    let (runtime, semantic, task) = pending.pop().unwrap();
    drop(task); // A fixture result below; never resolve live authentication or HTTP.
    assert_eq!(semantic, detail);
    let owner = crate::tests::jira_mut(&mut host).details[&detail_key("acme", "ABC-12")]
        .request
        .unwrap();
    all.extend(records(&press(&mut host, KeyCode::Esc)));
    let reopened = press(&mut host, KeyCode::Enter);
    let got = records(&reopened);
    let (again, _, _) = open_begin(&got, "jira.page.open");
    assert!(
        tasks(reopened).is_empty(),
        "in-flight fetch stays suppressed"
    );
    assert_eq!(
        crate::tests::jira_mut(&mut host).details[&detail_key("acme", "ABC-12")].request,
        Some(owner)
    );
    all.extend(got);
    all.extend(records(&host.update(Event::Tracked {
        action: runtime,
        semantic: Some(detail.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(JiraMsg::Detail {
                site: "acme".into(),
                key: "ABC-12".into(),
                action: scope,
                read: owner,
                result: Err("SECRET_ERROR".into()),
            }),
        }),
    })));
    assert_eq!(terminal(&all, &detail), [Outcome::Failed]);
    assert_eq!(terminal(&all, &opened), [Outcome::Succeeded]);
    assert_eq!(terminal(&all, &again), [Outcome::Succeeded]);
    assert_eq!(crate::tests::jira_mut(&mut host).pages.len(), 1);
    assert!(!crate::tests::jira_mut(&mut host).details[&detail_key("acme", "ABC-12")].loading);
    export(all, true);
}

struct ShowSender;

impl Plugin for ShowSender {
    fn id(&self) -> &'static str {
        "sender"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Show(*msg.downcast::<radar_core::Subject>().unwrap())]
    }
}

#[test]
fn cross_part_show_causes_one_open_with_resolved_server_scope_per_push() {
    let mut p = demo::sample();
    crate::tests::complete_detail(&mut p, "acme", "ABC-12", Ok(crate::tests::detail()));
    crate::tests::complete_detail(&mut p, "beta", "ABC-12", Ok(crate::tests::detail()));
    let mut host = Host::new(
        vec![Box::new(demo::sample()), Box::new(ShowSender)],
        demo::NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host.enable_journal();
    let mut all = vec![];
    let mut scopes = vec![];
    for site in ["acme", "beta"] {
        let subject = radar_core::Subject::new("ticket", "ABC-12", "SECRET_LABEL")
            .with("site", site)
            .with("title", "SECRET_TITLE");
        let shown = host.update(Event::Msg {
            plugin: 1,
            msg: Box::new(subject),
        });
        let got = records(&shown);
        let (show, _, _) = open_begin(&got, "ui.show");
        let (open, scope, parent) = open_begin(&got, "jira.page.open");
        assert_eq!(parent, Some(show));
        assert_eq!(terminal(&got, &open), [Outcome::Succeeded]);
        assert!(tasks(shown).is_empty());
        scopes.push(scope.subjects[0].key.clone());
        all.extend(got);
    }
    assert_eq!(scopes[0].id, scopes[1].id);
    assert_ne!(scopes[0].instance, scopes[1].instance);
    assert_eq!(crate::tests::jira_mut(&mut host).pages.len(), 2);
    let rejected = host.update(Event::Msg {
        plugin: 1,
        msg: Box::new(radar_core::Subject::new(
            "ticket",
            "SECRET_INVALID",
            "SECRET_LABEL",
        )),
    });
    assert!(!records(&rejected).iter().any(|record| matches!(record,
        JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.page.open")));
    assert_eq!(crate::tests::jira_mut(&mut host).pages.len(), 2);
    all.extend(records(&rejected));
    export(all, true);
}

#[test]
fn reversed_replies_keep_original_scopes_outcomes_and_write_siblings_isolated() {
    let plugin = Arc::new(Mutex::new(demo::sample()));
    let mut second = site();
    second.url = "https://other.example".into();
    let mut third = site();
    third.url = "https://third.example".into();
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
            reads: vec![
                (site(), Arc::new(rich())),
                (
                    second,
                    Arc::new(Routed::complete().with(WEB, vec![Reply::Network])),
                ),
                (
                    third,
                    Arc::new(Routed::complete().with(COMMENTS, vec![Reply::Network])),
                ),
            ],
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let start = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let mut all = records(&start);
    let write = all
        .iter()
        .find_map(|r| match r {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    let pending = tasks(start);
    let semantics = pending
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), 3);
    assert_ne!(semantics[0], semantics[1]);
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    assert!(semantics.iter().all(|id| terminal(&all, id).is_empty()));
    let notice_parent = all
        .iter()
        .find_map(|r| match r {
            JournalRecord::Begin {
                descriptor, parent, ..
            } if descriptor.kind.as_str() == "ui.notice" => parent.clone(),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        notice_parent, write,
        "a following sibling never joins one page read"
    );
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for (index, (runtime, semantic, task)) in pending.into_iter().enumerate().rev() {
        let got = records(&host.update(Event::Tracked {
            action: runtime.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: block(task),
            }),
        }));
        assert_eq!(
            terminal(&got, &semantic),
            [[Outcome::Succeeded, Outcome::Partial, Outcome::Failed][index]]
        );
        assert_eq!(
            terminal(&got, &runtime),
            [[Outcome::Succeeded, Outcome::Partial, Outcome::Failed][index]],
            "the task report belongs only to this runtime and semantic action"
        );
        assert!(terminal(&got, &write).is_empty());
        assert!(
            semantics
                .iter()
                .filter(|id| *id != &semantic)
                .all(|id| terminal(&got, id).is_empty())
        );
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
            {
                assert_eq!(parent.as_ref(), Some(&runtime));
                assert_eq!(descriptor.subjects.len(), 1);
                assert_eq!(descriptor.subjects[0].key.id, "ABC-12");
                assert_eq!(
                    descriptor.subjects[0].key.instance,
                    [
                        "https://jira.example/",
                        "https://other.example/",
                        "https://third.example/"
                    ][index]
                );
            }
        }
        all.extend(got);
    }
    let held = plugin.lock().unwrap();
    assert!(held.cfg.sites.is_empty());
    assert!(held.details.is_empty(), "late replies cannot recreate a removed connection's dedicated cache");
    drop(held);
    let events = export(all, false);
    let skipped: Vec<_> = events.iter().filter(|event|
        event.kind.as_str() == "jira.page.detail.application.skipped"
            && event.outcome == Outcome::Blocked).collect();
    assert_eq!(skipped.len(), 3);
    for event in skipped {
        assert_eq!(event.origin, Origin::Plugin("jira".into()));
        assert!(events.iter().any(|begin| begin.action_id == event.action_id && begin.metadata.count == Some(1)));
        assert_eq!(event.subjects[0].key.id, "ABC-12");
        assert!(event.parent_action_id.is_some());
    }

    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.page.detail.comments"
                && event.metadata.count == Some(2))
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.page.detail.web.errors"
                && event.outcome == Outcome::Failed)
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.page.detail.errors"
                && event.outcome == Outcome::Failed)
    );
}

#[test]
fn interrupted_join_has_unknown_history_without_completed_component_progress() {
    use std::task::{Context, Poll, Waker};
    let transport = Arc::new(Routed::complete().with(COMMENTS, vec![Reply::Pending]));
    let plugin = Arc::new(Mutex::new(demo::sample()));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
            reads: vec![(site(), transport.clone())],
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let start = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let all = records(&start);
    let (runtime, semantic, mut task) = tasks(start).pop().unwrap();
    assert!(matches!(
        task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    assert_eq!(
        transport.paths().len(),
        4,
        "issue and joined reads began, but no code request"
    );
    assert_eq!(transport.count(SUMMARY), 0);
    drop(task);
    assert!(terminal(&all, &runtime).is_empty());
    assert!(terminal(&all, &semantic).is_empty());
    let held = plugin.lock().unwrap();
    assert!(held.details[&detail_key("acme", "ABC-12")].loading);
    assert!(held.details[&detail_key("acme", "ABC-12")].value.is_none());
    drop(held);
    let events = export(all, true);
    assert!(events.iter().any(|event| event.action_id == semantic
        && event.outcome == Outcome::Unknown
        && event.metadata.reason == Some(Reason::NoTerminalObservation)));
    assert!(events.iter().any(|event| event.action_id == runtime
        && event.outcome == Outcome::Unknown
        && event.metadata.reason == Some(Reason::Interrupted)));
    assert!(
        !events
            .iter()
            .any(|event| event.kind.as_str().starts_with("jira.page.detail.")),
        "no result observations from an interrupted pipeline"
    );
}
