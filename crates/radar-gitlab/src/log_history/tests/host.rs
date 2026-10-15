use super::*;
use crate::client::{Req, Sent, Transport};
use radar_core::Host;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Origin, Query, SubjectRole};

struct Begin(Option<Vec<Resp>>);
struct BeginEarlier(Option<Vec<Resp>>);
struct Sibling;
struct Reads(GitlabPlugin);

struct PauseTrace;
impl Transport for PauseTrace {
    fn send(&self, req: &Req) -> Sent<'_> {
        if req.path.ends_with("/trace") {
            Box::pin(std::future::pending())
        } else {
            Box::pin(async { Ok(response(200, row(JOB))) })
        }
    }
}

impl Plugin for Reads {
    fn id(&self) -> &'static str {
        "gitlab"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        let msg = match msg.downcast::<Begin>() {
            Ok(begin_read) => {
                self.0.cfg.url = URL.into();
                let request = begin(&mut self.0, PROJECT, JOB);
                let task = match begin_read.0 {
                    Some(replies) => spawn(request, recorded(Replay::raw(replies))),
                    None => spawn(request, async {
                        read(&Client::over(Arc::new(PauseTrace)), PROJECT, JOB)
                            .await
                            .map_err(|e| format!("{e:#}"))
                    }),
                };
                return vec![
                    Effect::Journal(ActionDescriptor::new(
                        "gitlab.test.parent",
                        Origin::Plugin("gitlab".into()),
                    )),
                    Effect::Report(Outcome::Succeeded),
                    task,
                    Effect::SpawnAction {
                        action: ActionDescriptor::new(
                            "gitlab.test.sibling",
                            Origin::Plugin("gitlab".into()),
                        ),
                        task: Box::pin(async { Box::new(Sibling) as Msg }),
                    },
                ];
            }
            Err(msg) => msg,
        };
        let msg = match msg.downcast::<BeginEarlier>() {
            Ok(begin_read) => {
                let request = super::earlier::begin_earlier(&mut self.0, u64::MAX);
                let task = match begin_read.0 {
                    Some(replies) => earlier_spawn(
                        request,
                        super::earlier::earlier_read(Replay::raw(replies), u64::MAX),
                    ),
                    None => earlier_spawn(request, async {
                        Client::over(Arc::new(PauseTrace))
                            .trace_observed(PROJECT, JOB, Some(u64::MAX))
                            .await
                            .map_err(|e| format!("{e:#}"))
                    }),
                };
                let direct = begin(&mut self.0, "acme/other", JOB);
                return vec![
                    // A known successful write launches independent reads. No write is sent.
                    Effect::Journal(ActionDescriptor::new(
                        "gitlab.comment",
                        Origin::Plugin("gitlab".into()),
                    )),
                    Effect::Report(Outcome::Succeeded),
                    task,
                    spawn(direct, async {
                        read(
                            &Client::over(Replay::raw(vec![response(200, row(JOB)), tail(206)])),
                            "acme/other",
                            JOB,
                        )
                        .await
                        .map_err(|e| format!("{e:#}"))
                    }),
                    Effect::SpawnAction {
                        action: ActionDescriptor::new(
                            "gitlab.test.sibling",
                            Origin::Plugin("gitlab".into()),
                        ),
                        task: Box::pin(async { Box::new(Sibling) as Msg }),
                    },
                ];
            }
            Err(msg) => msg,
        };
        match msg.downcast::<Sibling>() {
            Ok(_) => vec![Effect::Report(Outcome::Failed)],
            Err(msg) => self.0.on_msg(msg),
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

fn descriptor(records: &[JournalRecord], id: &ActionId) -> ActionDescriptor {
    records
        .iter()
        .find_map(|r| match r {
            JournalRecord::Begin {
                id: got,
                descriptor,
                ..
            } if got == id => Some(descriptor.clone()),
            _ => None,
        })
        .unwrap()
}

fn terminal(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|r| match r {
            JournalRecord::Outcome {
                id: got, outcome, ..
            } if got == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}

type Pending = (ActionId, Option<ActionId>, radar_core::plugin::Task);
fn tasks(effects: Vec<HostEffect>) -> Vec<Pending> {
    effects
        .into_iter()
        .filter_map(|e| match e {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn persist(records: Vec<JournalRecord>) -> Vec<radar_core::journal::ActionEvent> {
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-log-history-{}",
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
            JournalRecord::Gap { .. } => panic!("unexpected gap"),
        }
    }
    journal.finish_session(&session).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let exported = serde_json::to_string(&events).unwrap();
    assert!(!exported.contains("SECRET"), "{exported}");
    assert!(
        events
            .iter()
            .flat_map(|e| &e.subjects)
            .all(|s| s.role == SubjectRole::Primary && s.key.kind == "job")
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
    events
}

#[test]
fn real_host_isolates_combined_result_from_parent_and_sibling_and_serializes_safely() {
    for (metadata, status, expected) in [
        (row(JOB), 206, Outcome::Succeeded),
        (Value::Null, 416, Outcome::Partial),
        (row(JOB), 500, Outcome::Failed),
    ] {
        let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
        host.enable_journal();
        let initial = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin(Some(vec![response(200, metadata), tail(status)]))),
        });
        let mut all = records(&initial);
        let parent = all
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "gitlab.test.parent" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(terminal(&all, &parent), [Outcome::Succeeded]);
        let mut prior = vec![];
        for (id, semantic, task) in tasks(initial) {
            let semantic = semantic.unwrap();
            let scope = descriptor(&all, &semantic);
            let is_log = scope.kind.as_str() == KIND;
            let fx = host.update(Event::Tracked {
                action: id.clone(),
                semantic: Some(semantic.clone()),
                event: Box::new(Event::Msg {
                    plugin: 0,
                    msg: block(task),
                }),
            });
            let got = records(&fx);
            assert_eq!(
                terminal(&got, &semantic),
                [if is_log { expected } else { Outcome::Failed }]
            );
            assert!(terminal(&got, &parent).is_empty());
            for other in &prior {
                assert!(terminal(&got, other).is_empty());
            }
            for record in &got {
                if let JournalRecord::Begin {
                    descriptor, parent, ..
                } = record
                    && descriptor.kind.as_str().starts_with("gitlab.job.log.")
                {
                    assert_eq!(parent.as_ref(), Some(&id));
                    assert_eq!(descriptor.subjects, scope.subjects);
                }
            }
            prior.push(semantic);
            all.extend(got);
        }
        let events = persist(all);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind.as_str() == KIND && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == parent && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [Outcome::Succeeded]
        );
    }
}

#[test]
fn interruption_after_metadata_leaves_unknown_without_invented_read_progress() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let initial = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin(None)),
    });
    let all = records(&initial);
    let mut found = false;
    for (_, semantic, mut task) in tasks(initial) {
        let semantic = semantic.unwrap();
        if descriptor(&all, &semantic).kind.as_str() != KIND {
            continue;
        }
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(task.as_mut().poll(&mut context).is_pending());
        assert!(terminal(&all, &semantic).is_empty());
        found = true;
    }
    assert!(found);
    let events = persist(all);
    assert!(
        events
            .iter()
            .any(|e| e.kind.as_str() == KIND && e.outcome == Outcome::Unknown)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str().starts_with("gitlab.job.log."))
    );
}

#[test]
fn earlier_success_and_failure_cannot_relabel_successful_write_direct_read_or_sibling() {
    for (status, expected) in [
        (206, Outcome::Succeeded),
        (416, Outcome::Succeeded),
        (500, Outcome::Failed),
    ] {
        let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
        host.enable_journal();
        let initial = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(BeginEarlier(Some(vec![tail(status)]))),
        });
        let mut all = records(&initial);
        let write = all
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "gitlab.comment" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
        let mut finished = vec![write.clone()];
        for (id, semantic, task) in tasks(initial).into_iter().rev() {
            let semantic = semantic.unwrap();
            let scope = descriptor(&all, &semantic);
            let expected = match scope.kind.as_str() {
                "gitlab.job.log.earlier" => expected,
                KIND => Outcome::Succeeded,
                _ => Outcome::Failed,
            };
            let fx = host.update(Event::Tracked {
                action: id.clone(),
                semantic: Some(semantic.clone()),
                event: Box::new(Event::Msg {
                    plugin: 0,
                    msg: block(task),
                }),
            });
            let got = records(&fx);
            assert_eq!(terminal(&got, &semantic), [expected]);
            for prior in &finished {
                assert!(terminal(&got, prior).is_empty());
            }
            for r in &got {
                if let JournalRecord::Begin {
                    descriptor, parent, ..
                } = r
                    && descriptor.kind.as_str().starts_with("gitlab.job.log.")
                {
                    assert_eq!(parent.as_ref(), Some(&id));
                    assert_eq!(descriptor.subjects, scope.subjects);
                    assert!(!descriptor.metadata.truncated);
                }
            }
            finished.push(semantic);
            all.extend(got);
        }
        let events = persist(all);
        for (kind, outcome) in [
            ("gitlab.comment", Outcome::Succeeded),
            (KIND, Outcome::Succeeded),
            ("gitlab.job.log.earlier", expected),
            ("gitlab.test.sibling", Outcome::Failed),
        ] {
            let terminal: Vec<_> = events
                .iter()
                .filter(|e| e.kind.as_str() == kind && e.outcome.is_terminal())
                .collect();
            assert_eq!(terminal.len(), 1);
            assert_eq!(terminal[0].outcome, outcome);
            if kind == "gitlab.job.log.earlier" {
                let requested = events
                    .iter()
                    .find(|e| {
                        e.action_id == terminal[0].action_id && e.outcome == Outcome::Requested
                    })
                    .unwrap();
                assert_eq!(requested.metadata.count, Some(u32::MAX));
                assert!(requested.metadata.truncated);
                assert_eq!(terminal[0].metadata.count, None);
                assert!(!terminal[0].metadata.truncated);
            }
        }
    }
}

#[test]
fn earlier_interruption_leaves_unknown_with_requested_boundary_and_no_result_facts() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let initial = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(BeginEarlier(None)),
    });
    let all = records(&initial);
    let mut found = false;
    for (_, semantic, mut task) in tasks(initial) {
        let semantic = semantic.unwrap();
        if descriptor(&all, &semantic).kind.as_str() != "gitlab.job.log.earlier" {
            continue;
        }
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(task.as_mut().poll(&mut context).is_pending());
        assert!(terminal(&all, &semantic).is_empty());
        found = true;
    }
    assert!(found);
    let events = persist(all);
    assert!(
        events
            .iter()
            .any(|e| e.kind.as_str() == "gitlab.job.log.earlier" && e.outcome == Outcome::Unknown)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str().starts_with("gitlab.job.log.earlier."))
    );
}
