use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, MOST_NUDGES, Method};
use crate::{GitlabPlugin, demo};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

const SOURCE: &str = "acme/team-a";
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";

fn page(iid: u64, more: bool) -> (u16, Value) {
    (
        200,
        json!({"data": {
            "currentUser": {"username": "SECRET_USERNAME"},
            "group": {"mergeRequests": {
                "nodes": if iid == 0 { vec![] } else { vec![json!({
                    "iid": iid, "title": "SECRET_TITLE", "description": "SECRET_BODY",
                    "project": {"fullPath": "acme/team-a/api", "name": "SECRET_PROJECT"}
                })] },
                "pageInfo": {"hasNextPage": more, "endCursor": "SECRET_CURSOR"}
            }}
        }}),
    )
}

fn incremental() -> Want {
    Want {
        changed_since: Some("SECRET_SINCE".into()),
        repo: Some(false),
        ..Default::default()
    }
}

fn observations(effects: &[Effect]) -> Vec<(&str, Option<u32>, Outcome, bool)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count,
                *outcome,
                action.metadata.truncated,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn fourth_page_distinguishes_complete_results_from_a_real_cutoff() {
    for more in [false, true] {
        let replay = Replay::new((1..=4).map(|i| page(i, i < 4 || more)).collect());
        let want = incremental();
        let got = block(Client::over(replay.clone()).sync(SOURCE, &want)).unwrap();
        assert_eq!(got.mrs.len(), 4);
        assert_eq!(got.mrs_truncated, more);
        assert_eq!(replay.asked().len(), 4, "the request bound is unchanged");
        let expected = if more {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        let fx = observed(&action(URL, SOURCE, &want), &Ok(got));
        assert!(matches!(fx[0], Effect::Report(outcome) if outcome == expected));
        assert_eq!(
            observations(&fx),
            [("gitlab.sync.merge_requests", Some(4), expected, more)]
        );
    }
}

#[test]
fn an_ended_cutoff_survives_a_later_complete_state() {
    let mut replies = vec![page(1, false)];
    replies.extend((2..=5).map(|i| page(i, true)));
    replies.push(page(6, false));
    let replay = Replay::new(replies);
    let want = Want {
        ended_since: Some("SECRET_WINDOW".into()),
        repo: Some(false),
        ..Default::default()
    };
    let got = block(Client::over(replay.clone()).sync(SOURCE, &want)).unwrap();
    assert!(got.mrs_truncated);
    assert_eq!(got.mrs.len(), 6);
    let states: Vec<_> = replay
        .asked()
        .iter()
        .map(|r| r.body.as_ref().unwrap()["variables"]["state"].clone())
        .collect();
    assert_eq!(
        states,
        ["opened", "merged", "merged", "merged", "merged", "closed"]
    );
}

/// A recheck asks each merge request by project and iid: there is no page
/// to cut off, and what it brings back counts with the merge requests.
#[test]
fn a_recheck_by_project_is_complete_and_counted() {
    let rechecked = json!({"data": {
        "m0": {"mergeRequest": {"headPipeline": {"status": "SUCCESS"}}},
        "m1": {"mergeRequest": {"headPipeline": {"status": "FAILED"}}}
    }});
    let replay = Replay::new(vec![page(0, false), (200, rechecked)]);
    let want = Want {
        recheck: (1..=2).map(|iid| ("acme/team-a/api".into(), iid)).collect(),
        ..incremental()
    };
    let got = block(Client::over(replay.clone()).sync(SOURCE, &want)).unwrap();
    assert!(!got.mrs_truncated);
    assert_eq!(replay.asked().len(), 2);
    let fx = observed(&action(URL, SOURCE, &want), &Ok(got));
    assert_eq!(
        observations(&fx),
        [("gitlab.sync.merge_requests", Some(2), Outcome::Succeeded, false)]
    );
}

#[test]
fn nudge_limits_and_errors_do_not_make_returned_lists_incomplete() {
    let mut replies = vec![(200, json!({})); MOST_NUDGES];
    replies[2] = (404, json!({"message": "SECRET_ERROR"}));
    replies.push(page(0, false));
    let replay = Replay::new(replies);
    let want = Want {
        nudge: (1..=11)
            .map(|iid| ("acme/team-a/api".into(), iid))
            .collect(),
        ..incremental()
    };
    let got = block(Client::over(replay.clone()).sync(SOURCE, &want)).unwrap();
    assert_eq!(
        (got.nudges.attempted, got.nudges.errors, got.nudges.deferred),
        (10, 1, 1)
    );
    assert!(!got.mrs_truncated);
    let asked = replay.asked();
    assert_eq!(asked.len(), 11);
    for (i, request) in asked.iter().take(10).enumerate() {
        assert_eq!(request.method, Method::Get);
        assert_eq!(
            request.path,
            format!(
                "/api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/{}?with_merge_status_recheck=true",
                i + 1
            )
        );
    }
    let fx = observed(&action(URL, SOURCE, &want), &Ok(got));
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert_eq!(
        observations(&fx),
        [
            (
                "gitlab.sync.merge_requests",
                Some(0),
                Outcome::Succeeded,
                false
            ),
            (
                "gitlab.sync.nudges.attempted",
                Some(10),
                Outcome::Succeeded,
                false
            ),
            ("gitlab.sync.nudges.errors", Some(1), Outcome::Failed, false),
            (
                "gitlab.sync.nudges.deferred",
                Some(1),
                Outcome::Blocked,
                false
            ),
        ]
    );
    let full = Want {
        changed_since: None,
        ..want
    };
    let replay = Replay::new(vec![page(0, false)]);
    let got = block(Client::over(replay.clone()).sync(SOURCE, &full)).unwrap();
    assert_eq!(
        got.nudges,
        Default::default(),
        "full sync does not dispatch nudges"
    );
    assert_eq!(replay.asked().len(), 1);
}

#[test]
fn project_absence_empty_and_late_error_are_different_evidence() {
    for projects in [false, true] {
        let mut replies = vec![page(0, false)];
        if projects {
            replies.push((200, json!({"data": {"group": {"projects": {"nodes": [], "pageInfo": {"hasNextPage": false}}}}})));
        }
        let want = Want {
            projects,
            ..incremental()
        };
        let got = block(Client::over(Replay::new(replies)).sync(SOURCE, &want)).unwrap();
        assert_eq!(got.projects, projects.then(Vec::new));
        let fx = observed(&action(URL, SOURCE, &want), &Ok(got));
        assert_eq!(observations(&fx).len(), if projects { 2 } else { 1 });
        if projects {
            assert_eq!(
                observations(&fx)[1],
                ("gitlab.sync.projects", Some(0), Outcome::Succeeded, false)
            );
        }
    }
    let want = Want {
        projects: true,
        ..incremental()
    };
    let result = block(
        Client::over(Replay::new(vec![
            page(1, false),
            (200, json!({"errors": [{"message": "SECRET_ERROR"}]})),
        ]))
        .sync(SOURCE, &want),
    )
    .map_err(|e| e.to_string());
    assert!(result.is_err());
    let fx = observed(&action(URL, SOURCE, &want), &result);
    assert!(matches!(fx[0], Effect::Report(Outcome::Failed)));
    assert_eq!(
        observations(&fx),
        [("gitlab.sync.errors", Some(1), Outcome::Failed, false)]
    );
}

#[test]
fn real_task_reply_keeps_original_scope_and_counts_received_rows_before_merge() {
    let mut plugin = demo::sample();
    let held = plugin.data[SOURCE].mrs.len();
    let want = incremental();
    let requested = want.clone();
    let client = Client::over(Replay::new(vec![page(900, false)]));
    let effect = spawn(URL, SOURCE.into(), want, async move {
        let result = client
            .sync(SOURCE, &requested)
            .await
            .map_err(|e| e.to_string());
        (result, Default::default())
    });
    let Effect::SpawnAction { action, task } = effect else {
        panic!("scoped task")
    };
    assert_eq!(action.subjects[0].key.instance, "https://git.example/");
    plugin.cfg.url = "https://changed.example".into();
    plugin.cfg.sources.clear();
    plugin.sync.in_flight = 1;
    let fx = plugin.on_msg(block(task));
    assert_eq!(plugin.sync.in_flight, 0);
    assert!(plugin.data[SOURCE].mrs.len() > 1);
    assert!(held > 1);
    let cache = plugin.snapshot().unwrap().to_string();
    assert!(!cache.contains("mrs_truncated"));
    assert!(!cache.contains("nudges"));
    assert_eq!(
        observations(&fx),
        [(
            "gitlab.sync.merge_requests",
            Some(1),
            Outcome::Succeeded,
            false
        )]
    );
    for effect in fx {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.instance, "https://git.example/");
            assert_eq!(action.subjects[0].key.id, SOURCE);
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
}

#[test]
fn unsafe_identities_are_omitted_and_reconfiguration_does_not_invent_a_subject() {
    for (url, group) in [
        ("not a URL", SOURCE),
        (URL, "acme/private?token=SECRET"),
        ("https://git.example/password=SECRET", SOURCE),
    ] {
        let scope = action(url, group, &Want::default());
        assert!(scope.subjects.is_empty());
        assert!(scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
    }
}

enum Begin {
    Start,
    Reconfigure,
}
struct Reads(GitlabPlugin);
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
    fn on_msg(&mut self, message: Msg) -> Vec<Effect> {
        match message.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    self.0.cfg.url = URL.into();
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "gitlab.test.write",
                        Origin::Plugin("gitlab".into()),
                    ))];
                    effects.extend(self.0.refresh());
                    effects.push(Effect::Notify("SECRET_NOTICE".into()));
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    self.0.cfg.url = "https://changed.example".into();
                    self.0.cfg.sources.clear();
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

#[test]
fn out_of_order_results_close_only_their_scope_and_export_no_payloads() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let tasks: Vec<_> = effects
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
        .collect();
    let mut all = records(&effects);
    assert_eq!(tasks.len(), 2);
    assert_ne!(tasks[0].1, tasks[1].1);
    let scopes: Vec<_> = tasks
        .iter()
        .map(|task| {
            all.iter()
                .find_map(|record| match record {
                    JournalRecord::Begin { id, descriptor, .. } if id == &task.1 => {
                        Some(descriptor.clone())
                    }
                    _ => None,
                })
                .unwrap()
        })
        .collect();
    let write = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "gitlab.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    for task in &tasks {
        assert!(terminal(&all, &task.1).is_empty());
    }
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for index in [1, 0] {
        let result = if index == 1 {
            Err("SECRET_ERROR".into())
        } else {
            let mut mr = crate::model::fixtures::mr(901, "SECRET_USERNAME", "acme/team-a/api");
            mr.title = "SECRET_TITLE".into();
            mr.project_name = "SECRET_PROJECT".into();
            mr.last_comment = Some(crate::model::Comment {
                author: "SECRET_AUTHOR".into(),
                body: "SECRET_BODY".into(),
                at: "SECRET_DATE".into(),
            });
            Ok(Synced {
                me: "SECRET_USERNAME".into(),
                mrs: vec![mr],
                projects: Some(vec![]),
                ..Default::default()
            })
        };
        let message = GlMsg::Loaded {
            group: scopes[index].subjects[0].key.id.clone(),
            want: Want::default(),
            action: scopes[index].clone(),
            result,
            net: Default::default(),
        };
        let got = records(&host.update(Event::Tracked {
            action: tasks[index].0.clone(),
            semantic: Some(tasks[index].1.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(message),
            }),
        }));
        assert_eq!(
            terminal(&got, &tasks[index].1),
            [if index == 1 {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            }]
        );
        assert!(terminal(&got, &tasks[1 - index].1).is_empty());
        assert!(terminal(&got, &write).is_empty());
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && matches!(
                    descriptor.kind.as_str(),
                    "cache.save" | "ui.changed" | "ui.notice"
                )
            {
                assert_eq!(
                    parent.as_ref(),
                    Some(&tasks[index].0),
                    "summaries must not capture later effects"
                );
            }
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && descriptor.kind.as_str().starts_with("gitlab.sync.")
            {
                assert_eq!(parent.as_ref(), Some(&tasks[index].0));
                assert_eq!(descriptor.subjects[0].key.instance, "https://git.example/");
                assert_eq!(
                    descriptor.subjects[0].key.id,
                    scopes[index].subjects[0].key.id
                );
            }
        }
        all.extend(got);
    }
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-sync-history-{}",
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
            JournalRecord::Gap { .. } => panic!("unexpected history gap"),
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
    assert!(!exported.contains("changed.example"));
    assert!(events.iter().any(
        |event| event.kind.as_str() == "gitlab.sync.errors" && event.metadata.count == Some(1)
    ));
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}
