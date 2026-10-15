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
    reads: Vec<(Site, usize, Arc<Recorded>)>,
    with_detail: bool,
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
                    if self.with_detail {
                        let mut p = self.plugin.lock().unwrap();
                        let detail_effects = page::fetch(&mut p, SITE, KEY, true);
                        let [Effect::SpawnAction { action, .. }] = &detail_effects[..] else {
                            panic!("detail read")
                        };
                        let read = p.details[&detail_key(SITE, KEY)].request.unwrap();
                        fx.push(crate::page_history::spawn(
                            action.clone(),
                            SITE.into(),
                            KEY.into(),
                            read,
                            async {
                                Ok(crate::client::DetailRead {
                                    detail: crate::tests::detail(),
                                    ..Default::default()
                                })
                            },
                        ));
                    }
                    for (mut site, start, transport) in self.reads.drain(..) {
                        let (scope, owner) = {
                            let mut p = self.plugin.lock().unwrap();
                            site.name = SITE.into();
                            p.cfg.sites = vec![site];
                            p.pages.clear();
                            page::open(&mut p, SITE, KEY);
                            let mut detail = crate::tests::detail();
                            detail.comments = vec![Comment::from_json(&comment()); start];
                            detail.comments_total = start + 100;
                            crate::tests::complete_detail(&mut p, SITE, KEY, Ok(detail));
                            let effects = request(&mut p);
                            let [Effect::SpawnAction { action, .. }] = &effects[..] else {
                                panic!("production comments dispatch")
                            };
                            (action.clone(), owner(&p))
                        };
                        fx.push(spawn(scope, SITE.into(), KEY.into(), owner, async move {
                            Client::over(transport)
                                .comments_observed(KEY, start, COMMENTS_PAGE)
                                .await
                                .map_err(|e| format!("{e:#}"))
                        }));
                    }
                    fx.push(Effect::Notify("SECRET_NOTICE".into()));
                    fx.push(Effect::Report(Outcome::Succeeded));
                    fx
                }
                Begin::Reconfigure => {
                    let mut p = self.plugin.lock().unwrap();
                    p.cfg.sites.clear();
                    crate::write_context::reconcile(&mut p);
                    vec![]
                }
            },
            Err(msg) => self.plugin.lock().unwrap().on_msg(msg),
        }
    }
}
fn records(fx: &[HostEffect]) -> Vec<JournalRecord> {
    fx.iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}
fn tasks(fx: Vec<HostEffect>) -> Vec<(ActionId, ActionId, Task)> {
    fx.into_iter()
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
fn export(records: Vec<JournalRecord>, finish: bool) -> Vec<ActionEvent> {
    let dir = std::env::temp_dir().join(format!(
        "radar-jira-comments-history-{}",
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
    let serialized = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "SECRET",
        "secret.example",
        "author",
        "created",
        "body",
        "token=",
        "read_id",
        "revision",
        "generation",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "{forbidden} leaked: {serialized}"
        );
    }
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
    events
}
fn complete(
    host: &mut Host,
    runtime: &ActionId,
    semantic: &ActionId,
    msg: Msg,
) -> Vec<JournalRecord> {
    records(&host.update(Event::Tracked {
        action: runtime.clone(),
        semantic: Some(semantic.clone()),
        event: Box::new(Event::Msg { plugin: 0, msg }),
    }))
}
fn begin_id(all: &[JournalRecord], kind: &str) -> ActionId {
    all.iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if descriptor.kind.as_str() == kind => {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn reversed_sites_keep_captured_offsets_actual_outcomes_and_parent_detail_siblings_isolated() {
    let plugin = Arc::new(Mutex::new(installed(2)));
    let mut second = site();
    second.url = "https://second.example".into();
    let mut third = site();
    third.url = "https://third.example".into();
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
            with_detail: true,
            reads: vec![
                (
                    site(),
                    2,
                    Recorded::body(json!({"comments": [comment(), null], "total": 100})),
                ),
                (second, 7, Recorded::body(json!({"comments": [comment()]}))),
                (third, 12, Recorded::new(vec![Reply::Network])),
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
    let write = begin_id(&all, "jira.test.write");
    let detail = begin_id(&all, "jira.page.detail");
    let pending = tasks(start);
    assert_eq!(pending.len(), 4);
    let semantics: Vec<_> = pending
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect();
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
    assert_eq!(notice_parent, write);
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for (index, (runtime, semantic, task)) in pending.into_iter().enumerate().rev() {
        let got = complete(&mut host, &runtime, &semantic, block(task));
        let expected = [
            Outcome::Succeeded,
            Outcome::Succeeded,
            Outcome::Partial,
            Outcome::Failed,
        ][index];
        assert_eq!(terminal(&got, &semantic), [expected]);
        assert_eq!(terminal(&got, &runtime), [expected]);
        assert!(terminal(&got, &write).is_empty());
        assert!(
            semantics
                .iter()
                .filter(|id| *id != &semantic)
                .all(|id| terminal(&got, id).is_empty())
        );
        if index != 0 {
            assert!(terminal(&got, &detail).is_empty());
            let report_index = got.iter().position(|r| matches!(r, JournalRecord::Outcome { id, outcome, .. } if id == &semantic && *outcome == expected)).unwrap();
            let skip_index = got.iter().position(|r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.page.comments.application.skipped")).unwrap();
            assert!(report_index < skip_index);
            for record in &got {
                if let JournalRecord::Begin {
                    descriptor, parent, ..
                } = record
                {
                    assert_eq!(parent.as_ref(), Some(&runtime));
                    assert_eq!(descriptor.subjects.len(), 1);
                    assert_eq!(descriptor.subjects[0].key.id, KEY);
                    assert_eq!(
                        descriptor.subjects[0].key.instance,
                        [
                            "https://jira.example/",
                            "https://second.example/",
                            "https://third.example/"
                        ][index - 1]
                    );
                }
            }
        }
        all.extend(got);
    }
    assert!(plugin.lock().unwrap().details.is_empty());
    let events = export(all, false);
    for (server, offset, outcome) in [
        ("https://jira.example/", 2, Outcome::Succeeded),
        ("https://second.example/", 7, Outcome::Partial),
        ("https://third.example/", 12, Outcome::Failed),
    ] {
        let begin = events
            .iter()
            .find(|event| {
                event.kind.as_str() == "jira.page.comments"
                    && event.subjects[0].key.instance == server
                    && event.metadata.count == Some(offset)
            })
            .unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.action_id == begin.action_id && event.outcome == outcome)
        );
        assert!(events.iter().any(|event| event.kind.as_str()
            == "jira.page.comments.application.skipped"
            && event.subjects[0].key.instance == server
            && event.outcome == Outcome::Blocked));
    }
}

#[test]
fn duplicate_delivery_keeps_actual_history_and_records_skip_without_clearing_new_owner() {
    let plugin = Arc::new(Mutex::new(installed(2)));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
            with_detail: false,
            reads: vec![(
                site(),
                2,
                Recorded::body(json!({"comments": [comment()], "total": 100})),
            )],
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
    let write = begin_id(&all, "jira.test.write");
    let (runtime, semantic, task) = tasks(start).pop().unwrap();
    let msg = block(task);
    let duplicate = {
        let got = msg.downcast_ref::<JiraMsg>().unwrap();
        let JiraMsg::More {
            site,
            key,
            action,
            read,
            ..
        } = got
        else {
            unreachable!()
        };
        Box::new(JiraMsg::More {
            site: site.clone(),
            key: key.clone(),
            action: action.clone(),
            read: *read,
            result: Ok(good()),
        }) as Msg
    };
    all.extend(complete(&mut host, &runtime, &semantic, msg));
    let before = {
        let mut p = plugin.lock().unwrap();
        request(&mut p);
        format!("{:?}", p.details)
    };
    let duplicate_records = complete(&mut host, &runtime, &semantic, duplicate);
    assert_eq!(
        terminal(&duplicate_records, &semantic),
        [Outcome::Succeeded]
    );
    assert!(terminal(&duplicate_records, &write).is_empty());
    let report = duplicate_records.iter().position(|r| matches!(r, JournalRecord::Outcome { id, outcome: Outcome::Succeeded, .. } if id == &semantic)).unwrap();
    let skip = duplicate_records.iter().position(|r| matches!(r, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "jira.page.comments.application.skipped")).unwrap();
    assert!(report < skip);
    assert_eq!(format!("{:?}", plugin.lock().unwrap().details), before);
    all.extend(duplicate_records);
    let events = export(all, false);
    assert_eq!(
        events
            .iter()
            .filter(
                |e| e.kind.as_str() == "jira.page.comments.application.skipped"
                    && e.outcome == Outcome::Blocked
            )
            .count(),
        1
    );
}

#[test]
fn interrupted_page_retains_requested_offset_and_unknown_without_inferred_result_progress() {
    use std::task::{Context, Poll, Waker};
    let transport = Recorded::new(vec![Reply::Pending]);
    let plugin = Arc::new(Mutex::new(installed(2)));
    let mut host = Host::new(
        vec![Box::new(Harness {
            plugin: plugin.clone(),
            with_detail: false,
            reads: vec![(site(), 19, transport.clone())],
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
    assert_eq!(transport.requests().len(), 1);
    assert!(
        transport.requests()[0]
            .path
            .contains("startAt=19&maxResults=50&orderBy=-created")
    );
    drop(task);
    assert!(terminal(&all, &runtime).is_empty());
    assert!(terminal(&all, &semantic).is_empty());
    assert!(
        plugin.lock().unwrap().details[&detail_key(SITE, KEY)]
            .more
            .is_some()
    );
    let events = export(all, true);
    assert!(
        events
            .iter()
            .any(|e| e.action_id == semantic && e.metadata.count == Some(19))
    );
    assert!(events.iter().any(|e| e.action_id == semantic
        && e.outcome == Outcome::Unknown
        && e.metadata.reason == Some(Reason::NoTerminalObservation)));
    assert!(events.iter().any(|e| e.action_id == runtime
        && e.outcome == Outcome::Unknown
        && e.metadata.reason == Some(Reason::Interrupted)));
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str().starts_with("jira.page.comments.")),
        "no result/gap/request-limit observations inferred on interruption"
    );
}
