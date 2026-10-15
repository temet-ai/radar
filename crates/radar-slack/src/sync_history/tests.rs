use super::*;
use crate::{SlackPlugin, demo};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::ActionId;
use radar_core::{Host, Plugin};

#[test]
fn workspace_and_channel_identity_is_validated_without_display_data() {
    let a = action("slack.sync.channel", "T000ACME1", demo::TEAM);
    assert_eq!(a.subjects[0].key.instance, "T000ACME1");
    assert_eq!(a.subjects[0].key.id, demo::TEAM);
    let b = action("slack.sync.channel", "T000NEXT1", demo::TEAM);
    assert_ne!(a.subjects[0].key, b.subjects[0].key);
    let long_team = "T0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let longer = action("slack.sync.channel", long_team, demo::TEAM);
    assert_eq!(longer.subjects[0].key.instance, long_team);
    for (team, channel) in [
        ("", demo::TEAM),
        ("T", demo::TEAM),
        ("T000ACME1\n", demo::TEAM),
        ("T000ACME1?token=SECRET", demo::TEAM),
        ("xoxp-SECRET", demo::TEAM),
        ("SECRET_WORKSPACE", demo::TEAM),
        ("T000ACME1", "SECRET_ALIAS"),
        ("T000ACME1", "C01234567?token=SECRET"),
        ("https://slack.com", demo::TEAM),
    ] {
        let got = action("slack.sync.channel", team, channel);
        assert!(got.subjects.is_empty());
        assert!(got.metadata.redacted);
        assert_eq!(got.metadata.omitted_subjects, 1);
        assert!(!format!("{got:?}").contains("SECRET"));
    }
}

#[test]
fn auxiliary_and_deferred_gaps_do_not_make_returned_message_lists_truncated() {
    let a = action("slack.sync.channel", "T000ACME1", demo::TEAM);
    let fx = channel(
        &a,
        &Ok(ChannelRead {
            info: None,
            list: MessagesRead {
                messages: vec![],
                evidence: Default::default(),
            },
        }),
        NamesEvidence {
            attempted: 1,
            errors: 1,
            deferred: 4,
        },
    );
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert!(
        matches!(&fx[1], Effect::Observe { action, outcome: Outcome::Succeeded }
        if action.metadata.count == Some(0) && !action.metadata.truncated)
    );
    let scope = action("slack.sync.threads", "T000ACME1", demo::TEAM);
    let fx = threads(
        &scope,
        &Ok(ThreadsRead {
            deferred: 1,
            ..Default::default()
        }),
        NamesEvidence::default(),
    );
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert!(
        matches!(&fx[2], Effect::Observe { action, outcome: Outcome::Succeeded }
        if action.kind.as_str() == "slack.sync.threads.replies" && !action.metadata.truncated)
    );
}

#[test]
fn thread_list_cutoffs_and_incomplete_pages_are_distinct_counted_units() {
    let scope = action("slack.sync.threads", "T000ACME1", demo::TEAM);
    let fx = threads(
        &scope,
        &Ok(ThreadsRead {
            threads: [("1700000000.000001".into(), vec![Message::default(); 3])].into(),
            evidence: ListEvidence {
                truncated: true,
                incomplete_pages: 5,
            },
            cutoffs: 2,
            deferred: 0,
            ..Default::default()
        }),
        NamesEvidence::default(),
    );
    let counts: Vec<_> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Observe { action, .. } => Some((action.kind.as_str(), action.metadata.count)),
            _ => None,
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("slack.sync.threads.returned", Some(1)),
            ("slack.sync.threads.replies", Some(3)),
            ("slack.sync.lists.truncated", Some(2)),
            ("slack.sync.pages.incomplete", Some(5)),
        ]
    );
}

enum Begin {
    Start,
    Reconfigure,
}
struct Reads(SlackPlugin);
impl Plugin for Reads {
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
        match message.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    self.0.cfg.channels.truncate(2);
                    for c in &mut self.0.cfg.channels {
                        c.watched = true;
                    }
                    self.0.via = crate::Via(Some(crate::client::Recorded::new(vec![])));
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "slack.test.write",
                        Origin::Plugin("slack".into()),
                    ))];
                    effects.extend(self.0.read());
                    effects.push(Effect::Notify("SECRET_NOTICE".into()));
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    self.0.data.me.as_mut().unwrap().team_id = "T000NEXT1".into();
                    self.0.cfg.channels.clear();
                    self.0.via = crate::Via(Some(crate::client::Recorded::new(vec![])));
                    vec![]
                }
            },
            Err(message) => {
                let mut effects = self.0.on_msg(message);
                effects.push(Effect::Notify("SECRET_FOLLOW_ON_NOTICE".into()));
                effects
            }
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

fn task_scopes(effects: &[HostEffect], kind: &str) -> Vec<(ActionId, ActionId, ActionDescriptor)> {
    let records = records(effects);
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                records.iter().find_map(|record| match record {
                    JournalRecord::Begin { id, descriptor, .. }
                        if id == semantic && descriptor.kind.as_str() == kind =>
                    {
                        Some((action.clone(), semantic.clone(), descriptor.clone()))
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .collect()
}

fn deliver(
    host: &mut Host,
    task: &(ActionId, ActionId, ActionDescriptor),
    read: Read,
) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: task.0.clone(),
        semantic: Some(task.1.clone()),
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(SlackMsg::Read {
                stamp: Default::default(),
                read: Box::new(read),
            }),
        }),
    })
}

#[test]
fn out_of_order_and_dependent_tasks_keep_captured_scopes_and_sqlite_has_no_payloads() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let tasks = task_scopes(&effects, "slack.sync.channel");
    assert_eq!(tasks.len(), 2);
    assert_ne!(tasks[0].1, tasks[1].1);
    let mut all = records(&effects);
    let write = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "slack.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    assert!(tasks.iter().all(|t| terminal(&all, &t.1).is_empty()));
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    let mut child = None;
    for index in [1, 0] {
        let result = if index == 1 {
            Err("SECRET_ERROR".into())
        } else {
            Ok(ChannelRead {
                info: Some(ChannelInfo {
                    name: "SECRET_CHANNEL".into(),
                    purpose: "SECRET_PURPOSE".into(),
                    ..Default::default()
                }),
                list: MessagesRead {
                    messages: vec![Message {
                        ts: "1700000000.000001".into(),
                        text: "SECRET_BODY".into(),
                        reply_count: 1,
                        latest_reply: Some("1700000001.000001".into()),
                        ..Default::default()
                    }],
                    evidence: Default::default(),
                },
            })
        };
        let effects = deliver(
            &mut host,
            &tasks[index],
            Read::Channel {
                id: tasks[index].2.subjects[0].key.id.clone(),
                result,
                names: vec![("SECRET_USER_ID".into(), "SECRET_NAME".into())],
                names_evidence: NamesEvidence::default(),
                action: tasks[index].2.clone(),
            },
        );
        let got = records(&effects);
        let expected = if index == 1 {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        };
        assert_eq!(terminal(&got, &tasks[index].0), [expected]);
        assert_eq!(terminal(&got, &tasks[index].1), [expected]);
        assert!(terminal(&got, &tasks[1 - index].1).is_empty());
        assert!(terminal(&got, &write).is_empty());
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
            {
                if descriptor.kind.as_str().starts_with("slack.sync.")
                    || matches!(
                        descriptor.kind.as_str(),
                        "cache.save" | "ui.changed" | "ui.notice"
                    )
                {
                    assert_eq!(
                        parent.as_ref(),
                        Some(&tasks[index].0),
                        "observations and following effects keep their actual task parent"
                    );
                }
                if descriptor.kind.as_str().starts_with("slack.sync.")
                    && descriptor.kind.as_str() != "slack.sync.threads"
                {
                    assert_eq!(descriptor.subjects, tasks[index].2.subjects);
                }
            }
        }
        if index == 0 {
            let children = task_scopes(&effects, "slack.sync.threads");
            assert_eq!(children.len(), 1);
            assert_eq!(
                children[0].2.subjects[0].key.instance, "T000NEXT1",
                "child captures its newly dispatched client scope"
            );
            assert!(terminal(&got, &children[0].1).is_empty());
            child = Some(children.into_iter().next().unwrap());
        }
        all.extend(got);
    }
    let child = child.unwrap();
    let effects = deliver(
        &mut host,
        &child,
        Read::Threads {
            id: child.2.subjects[0].key.id.clone(),
            result: Ok(ThreadsRead {
                threads: [(
                    "1700000000.000001".into(),
                    vec![Message {
                        text: "SECRET_REPLY".into(),
                        ..Default::default()
                    }],
                )]
                .into(),
                ..Default::default()
            }),
            names: vec![],
            names_evidence: NamesEvidence::default(),
            action: child.2.clone(),
        },
    );
    let got = records(&effects);
    assert_eq!(terminal(&got, &child.1), [Outcome::Succeeded]);
    assert!(tasks.iter().all(|t| terminal(&got, &t.1).is_empty()));
    assert!(terminal(&got, &write).is_empty());
    all.extend(got);
    assert!(!format!("{all:?}").contains("SECRET"));
    #[cfg(unix)]
    {
        use radar_core::journal::{Journal, Query};
        let dir = std::env::temp_dir().join(format!(
            "radar-slack-sync-history-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in all {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap(),
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
                .any(|e| e.kind.as_str() == "slack.sync.errors" && e.metadata.count == Some(1))
        );
        assert!(events.iter().any(
            |e| e.kind.as_str() == "slack.sync.threads.replies" && e.metadata.count == Some(1)
        ));
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

struct FollowReads(SlackPlugin);
impl Plugin for FollowReads {
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
        match message.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    self.0.cfg.channels.clear();
                    let ask = crate::model::Asked {
                        channel: demo::TEAM.into(),
                        ts: demo::ago(60),
                        ..Default::default()
                    };
                    self.0.data.asked = vec![ask.clone(), ask.clone()];
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "slack.test.write",
                        Origin::Plugin("slack".into()),
                    ))];
                    for team in ["T000ACME1", "T000NEXT1"] {
                        let mut me = self.0.data.me.clone().unwrap();
                        me.team_id = team.into();
                        assert!(self.0.install_me(me));
                        self.0.data.asked = vec![ask.clone(), ask.clone()];
                        self.0.via = crate::Via(Some(crate::client::Recorded::new(vec![])));
                        effects.extend(self.0.read());
                    }
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    let mut me = self.0.data.me.clone().unwrap();
                    me.team_id = "T000LAST1".into();
                    assert!(self.0.install_me(me));
                    self.0.data.asked.clear();
                    vec![]
                }
            },
            Err(message) => self.0.on_msg(message),
        }
    }
}

#[test]
fn followed_batches_keep_duplicate_target_evidence_and_out_of_order_causal_identity() {
    let mut host = Host::new(vec![Box::new(FollowReads(demo::sample()))], demo::NOW).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let tasks = task_scopes(&effects, "slack.sync.followed");
    assert_eq!(tasks.len(), 2);
    let mut all = records(&effects);
    assert!(!all.iter().any(|record| matches!(record, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "slack.sync.followed.thread")), "no target progress before batch completion");
    let write = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "slack.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    let ts = demo::ago(60);
    for index in [1, 0] {
        let team = if index == 0 { "T000ACME1" } else { "T000NEXT1" };
        let mut batch = FollowedBatch::default();
        for candidate in 0..2 {
            batch.targets.push(FollowedTarget {
                action: followed_target(team, demo::TEAM, &ts),
                response: (index == 0).then_some(FollowedResponse {
                    parent_found: candidate == 0,
                    messages: if candidate == 0 { 2 } else { 1 },
                    evidence: ListEvidence {
                        truncated: candidate != 0,
                        incomplete_pages: 0,
                    },
                }),
            });
        }
        if index == 0 {
            batch.updates.push((
                demo::TEAM.into(),
                ts.clone(),
                987654,
                vec![Reaction {
                    name: "SECRET_REACTION".into(),
                    users: vec!["SECRET_PERSON".into()],
                    ..Default::default()
                }],
            ));
        }
        let effects = deliver(
            &mut host,
            &tasks[index],
            Read::Followed {
                action: tasks[index].2.clone(),
                batch,
            },
        );
        let got = records(&effects);
        assert_eq!(
            terminal(&got, &tasks[index].1),
            [if index == 0 {
                Outcome::Partial
            } else {
                Outcome::Failed
            }]
        );
        assert!(terminal(&got, &tasks[1 - index].1).is_empty());
        assert!(terminal(&got, &write).is_empty());
        let targets: Vec<_> = got
            .iter()
            .filter_map(|record| match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if descriptor.kind.as_str() == "slack.sync.followed.thread" => {
                    Some((id, descriptor, parent))
                }
                _ => None,
            })
            .collect();
        assert_eq!(targets.len(), 2);
        assert_ne!(
            targets[0].0, targets[1].0,
            "duplicate candidates retain independent observation actions"
        );
        for (candidate, (id, descriptor, parent)) in targets.into_iter().enumerate() {
            assert_eq!(parent.as_ref(), Some(&tasks[index].0));
            assert_eq!(descriptor.subjects[0].key.instance, team);
            assert_eq!(descriptor.subjects[0].key.kind, "thread");
            assert_eq!(
                descriptor.subjects[0].key.id,
                format!("{}/{ts}", demo::TEAM)
            );
            assert_eq!(
                descriptor.metadata.count,
                if index == 0 {
                    Some(if candidate == 0 { 2 } else { 1 })
                } else {
                    None
                }
            );
            assert_eq!(
                terminal(&got, id),
                [if index == 0 && candidate == 0 {
                    Outcome::Succeeded
                } else {
                    Outcome::Failed
                }]
            );
            assert!(!got.iter().any(|record| matches!(record, JournalRecord::Outcome { id: at, outcome: Outcome::Started, .. } if at == id)), "completion observations invent no network start");
            if index == 0 && candidate == 1 {
                assert_eq!(descriptor.metadata.reason, Some(Reason::Unavailable));
                assert!(descriptor.metadata.truncated);
            }
        }
        all.extend(got);
    }
    assert!(!format!("{all:?}").contains("SECRET"));
    #[cfg(unix)]
    {
        use radar_core::journal::{Journal, Query};
        let dir = std::env::temp_dir().join(format!(
            "radar-slack-followed-history-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in all {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap(),
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
        let events = journal
            .query(&Query {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let exported = serde_json::to_string(&events).unwrap();
        assert!(!exported.contains("SECRET") && !exported.contains("987654"));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind.as_str() == "slack.sync.followed.thread"
                    && e.outcome == Outcome::Requested)
                .count(),
            4
        );
        assert!(
            !events
                .iter()
                .any(|e| e.kind.as_str() == "slack.sync.followed.thread"
                    && e.outcome == Outcome::Started)
        );
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn followed_subjects_use_workspace_and_canonical_thread_identity_without_unsafe_fallbacks() {
    let workspace = followed_action("T000ACME1");
    assert_eq!(
        workspace.subjects[0].key,
        SubjectKey::new("slack", "T000ACME1", "workspace", "workspace").unwrap()
    );
    let target = followed_target("T000ACME1", demo::TEAM, "1700000000.123456");
    assert_eq!(
        target.subjects[0].key,
        SubjectKey::new(
            "slack",
            "T000ACME1",
            "thread",
            &format!("{}/1700000000.123456", demo::TEAM)
        )
        .unwrap()
    );
    for (team, channel, ts) in [
        ("", demo::TEAM, "1.2"),
        ("T000ACME1", "SECRET_CHANNEL", "1.2"),
        ("T000ACME1", demo::TEAM, "SECRET_TIMESTAMP"),
        ("T000ACME1", demo::TEAM, "1."),
        ("T000ACME1", demo::TEAM, ".2"),
        ("T000ACME1", demo::TEAM, "1.2.3"),
        ("T000ACME1", demo::TEAM, "1.2?token=SECRET"),
        ("T000ACME1", demo::TEAM, "1.2\n"),
    ] {
        let target = followed_target(team, channel, ts);
        assert!(target.subjects.is_empty());
        assert!(target.metadata.redacted);
        assert_eq!(target.metadata.omitted_subjects, 1);
        assert!(!format!("{target:?}").contains("SECRET"));
    }
    assert!(followed_action("SECRET_TEAM").subjects.is_empty());
}

#[test]
fn for_you_results_keep_dispatch_scope_and_real_completed_evidence_through_host_and_sqlite() {
    let mut host = Host::new(vec![Box::new(FollowReads(demo::sample()))], demo::NOW).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let tasks = task_scopes(&effects, "slack.sync.for_you");
    assert_eq!(tasks.len(), 2);
    let mut all = records(&effects);
    assert!(
        !all.iter()
            .any(|r| matches!(r, JournalRecord::Begin { descriptor, .. }
        if descriptor.kind.as_str() == "slack.sync.for_you.message.observed"))
    );
    let write = all
        .iter()
        .find_map(|r| match r {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "slack.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for index in [1, 0] {
        let team = if index == 0 { "T000ACME1" } else { "T000NEXT1" };
        let found = crate::client::Found {
            channel_id: demo::TEAM.into(),
            channel_name: "SECRET_CHANNEL_NAME".into(),
            permalink: "https://private.example/SECRET_LINK?token=SECRET_TOKEN".into(),
            message: Message {
                ts: "1791000001.000001".into(),
                thread_ts: Some("1791000000.000001".into()),
                user: Some("SECRET_USER".into()),
                text: "SECRET_MESSAGE".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let evidence = ForYouEvidence {
            planned: 2,
            queries: vec![
                SearchObservation {
                    kind: SearchKind::Mentions,
                    list: Some(SearchList {
                        returned: 1,
                        evidence: ListEvidence::default(),
                    }),
                },
                SearchObservation {
                    kind: SearchKind::Direct,
                    list: (index == 0).then_some(SearchList {
                        returned: 0,
                        evidence: ListEvidence::default(),
                    }),
                },
            ],
            groups: GroupsEvidence::Returned {
                returned: 1,
                selected: 0,
                incomplete_collection: false,
                incomplete_entries: 1,
            },
            names: NamesEvidence {
                attempted: 1,
                errors: 0,
                deferred: 0,
            },
            unique: 1,
            retained: vec![for_you_message(team, &found)],
            ..Default::default()
        };
        let effects = deliver(
            &mut host,
            &tasks[index],
            Read::ForYou {
                action: tasks[index].2.clone(),
                groups: Some(vec![crate::client::UserGroup {
                    id: "SECRET_GROUP".into(),
                    ..Default::default()
                }]),
                names: vec![("SECRET_USER".into(), "SECRET_DISPLAY_NAME".into())],
                result: if index == 0 {
                    Ok(vec![found])
                } else {
                    Err("SECRET_SEARCH_ERROR".into())
                },
                evidence,
            },
        );
        let got = records(&effects);
        assert_eq!(
            terminal(&got, &tasks[index].1),
            [if index == 0 {
                Outcome::Partial
            } else {
                Outcome::Failed
            }]
        );
        assert!(terminal(&got, &tasks[1 - index].1).is_empty());
        assert!(terminal(&got, &write).is_empty());
        let (id, descriptor, parent) = got
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if descriptor.kind.as_str() == "slack.sync.for_you.message.observed" => {
                    Some((id, descriptor, parent))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(parent.as_ref(), Some(&tasks[index].0));
        assert_eq!(descriptor.metadata.count, Some(1));
        assert_eq!(descriptor.subjects.len(), 2);
        assert_eq!(descriptor.subjects[0].key.instance, team);
        assert_eq!(descriptor.subjects[0].key.kind, "message");
        assert_eq!(descriptor.subjects[0].role, SubjectRole::Result);
        assert_eq!(descriptor.subjects[1].key.instance, team);
        assert_eq!(descriptor.subjects[1].key.kind, "thread");
        assert_eq!(descriptor.subjects[1].role, SubjectRole::Context);
        assert_eq!(terminal(&got, id), [Outcome::Succeeded]);
        assert!(!got.iter().any(|r| matches!(r, JournalRecord::Outcome { id: at, outcome: Outcome::Started, .. } if at == id)));
        all.extend(got);
    }
    let safe = format!("{all:?}");
    assert!(!safe.contains("SECRET") && !safe.contains("T000LAST1"));
    #[cfg(unix)]
    {
        use radar_core::journal::{Journal, Query};
        let dir = std::env::temp_dir().join(format!(
            "radar-slack-for-you-history-{}",
            ActionId::new().as_str()
        ));
        let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
        let session = journal.start_session().unwrap();
        for record in all {
            match record {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } => journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap(),
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
        let events = journal
            .query(&Query {
                limit: 1000,
                ..Default::default()
            })
            .unwrap();
        let exported = serde_json::to_string(&events).unwrap();
        assert!(!exported.contains("SECRET") && !exported.contains("T000LAST1"));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind.as_str() == "slack.sync.for_you.message.observed"
                    && e.outcome == Outcome::Requested)
                .count(),
            2
        );
        assert!(
            !events
                .iter()
                .any(|e| e.kind.as_str() == "slack.sync.for_you.message.observed"
                    && e.outcome == Outcome::Started)
        );
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

// Read the authenticated synthetic account's partition, never an unowned cache.
pub(super) fn account_data(cache: &serde_json::Value) -> &serde_json::Value {
    assert_eq!(cache["version"], 1);
    &cache["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|partition| {
            partition["owner"]["team_id"] == "T000ACME1"
                && partition["owner"]["user_id"] == demo::ALICE
        })
        .expect("fixture account partition")["data"]
}
