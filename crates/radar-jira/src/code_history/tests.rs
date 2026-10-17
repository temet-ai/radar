use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, CodeEvidence};
use crate::{JiraPlugin, demo};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

fn site() -> Site {
    let mut site = demo::sample().cfg.sites.remove(0);
    site.name = "SECRET_ALIAS".into();
    site.url =
        "https://SECRET_USER:SECRET_PASSWORD@jira.example/?token=SECRET_TOKEN#SECRET_FRAGMENT"
            .into();
    site.email = "SECRET_EMAIL".into();
    site.token = "SECRET_TOKEN".into();
    site
}

fn summary(hosts: Value) -> (u16, Value) {
    (
        200,
        json!({"summary": {"pullrequest": {"byInstanceType": hosts}}}),
    )
}

fn link() -> Value {
    json!({
        "url": "https://elsewhere.example/reset/SECRET_INVITE?token=SECRET_TOKEN#SECRET_FRAGMENT",
        "id": "SECRET_REPO!42", "name": "SECRET_TITLE", "status": "SECRET_STATUS",
        "repositoryName": "SECRET_REPO", "source": {"branch": "SECRET_BRANCH"}
    })
}

fn detail(rows: Value) -> (u16, Value) {
    (200, json!({"detail": [{"pullRequests": rows}]}))
}

fn read(replies: Vec<(u16, Value)>) -> CodeRead {
    block(Client::over(Replay::new(replies)).code_observed("10012")).unwrap()
}

fn observations(effects: &[Effect]) -> Vec<(&str, Option<u32>, Outcome, Option<Reason>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count,
                *outcome,
                action.metadata.reason,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn explicit_empty_collections_are_successful_observed_zeroes() {
    for replies in [
        vec![summary(json!({}))],
        vec![summary(json!({"SECRET_APPLICATION": {"count": 0}}))],
        vec![
            summary(json!({"SECRET_APPLICATION": {"count": 10}})),
            detail(json!([])),
        ],
        vec![
            summary(json!({"SECRET_APPLICATION": {"count": 10}})),
            (200, json!({"detail": []})),
        ],
    ] {
        let got = read(replies);
        assert!(got.links.is_empty());
        assert_eq!(got.evidence, CodeEvidence::default());
        let effects = observed(&action(&site(), "ABC-12"), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        assert_eq!(
            observations(&effects),
            [("jira.sync.code.links", Some(0), Outcome::Succeeded, None)]
        );
    }
}

#[test]
fn wrapper_and_observed_read_preserve_request_order_duplicates_and_permissive_rows() {
    let rows = json!([link(), {"url": ""}, {"url": "not a validated URL"}, link()]);
    let replies = vec![
        summary(
            json!({"A SECRET": {"count": 1}, "B SECRET": {"count": 99}, "Z SECRET": {"count": 0}}),
        ),
        detail(rows),
        detail(json!([link()])),
    ];
    let old = Replay::new(replies.clone());
    let old_links = block(Client::over(old.clone()).code("10012")).unwrap();
    let replay = Replay::new(replies);
    let got = block(Client::over(replay.clone()).code_observed("10012")).unwrap();
    assert_eq!(old_links, got.links);
    assert_eq!(
        got.links.len(),
        5,
        "count returned entries, including duplicates and unvalidated URLs"
    );
    assert_eq!(got.evidence, CodeEvidence::default());
    assert_eq!(old.paths(), replay.paths());
    assert_eq!(
        replay.paths(),
        [
            "/rest/dev-status/latest/issue/summary?issueId=10012",
            "/rest/dev-status/latest/issue/detail?issueId=10012&applicationType=A%20SECRET&dataType=pullrequest",
            "/rest/dev-status/latest/issue/detail?issueId=10012&applicationType=B%20SECRET&dataType=pullrequest",
        ]
    );
    let effects = observed(&action(&site(), "ABC-12"), &Ok(got));
    assert_eq!(
        observations(&effects),
        [("jira.sync.code.links", Some(5), Outcome::Succeeded, None)]
    );
    for effect in effects {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(
                action.subjects.len(),
                1,
                "no derived MR or repository identities"
            );
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
}

#[test]
fn missing_or_malformed_summary_is_partial_without_extra_requests() {
    for body in [
        json!({}),
        json!({"summary": null}),
        json!({"summary": {"pullrequest": {"byInstanceType": []}}}),
        json!({"summary": {"pullrequest": {"byInstanceType": {"SECRET": {}}}}}),
        json!({"summary": {"pullrequest": {"byInstanceType": {"SECRET": {"count": "1"}}}}}),
        json!({"summary": {"pullrequest": {"byInstanceType": {"SECRET": {"count": -1}}}}}),
        json!({"summary": {"pullrequest": {"byInstanceType": {"SECRET": null, "OTHER": {}}}}}),
    ] {
        let replay = Replay::new(vec![(200, body)]);
        let got = block(Client::over(replay.clone()).code_observed("10012")).unwrap();
        assert_eq!(replay.paths().len(), 1);
        assert!(got.links.is_empty());
        assert_eq!(got.evidence.incomplete_responses, 1);
        assert_eq!(
            got.evidence.omitted_rows, 0,
            "skipped host counts are not omitted PR rows"
        );
        let effects = observed(&action(&site(), "ABC-12"), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
        assert_eq!(
            observations(&effects),
            [
                ("jira.sync.code.links", Some(0), Outcome::Partial, None),
                (
                    "jira.sync.code.responses.incomplete",
                    Some(1),
                    Outcome::Partial,
                    None
                ),
            ]
        );
    }
}

#[test]
fn mixed_valid_and_malformed_summary_hosts_keep_the_legacy_selection() {
    let replay = Replay::new(vec![
        summary(json!({"A": {}, "B": {"count": 1}, "C": {"count": "1"}, "D": {"count": 0}})),
        detail(json!([link()])),
    ]);
    let got = block(Client::over(replay.clone()).code_observed("10012")).unwrap();
    assert_eq!(got.links.len(), 1);
    assert_eq!(got.evidence.incomplete_responses, 1);
    assert_eq!(replay.paths().len(), 2);
    assert!(replay.paths()[1].contains("applicationType=B&"));
}

#[test]
fn incomplete_details_count_responses_and_omissions_count_only_rejected_rows() {
    for body in [
        json!({}),
        json!({"detail": null}),
        json!({"detail": "SECRET"}),
        json!({"detail": [{}, null, {"pullRequests": {}}]}),
    ] {
        let got = read(vec![summary(json!({"A": {"count": 1}})), (200, body)]);
        assert_eq!(got.evidence.incomplete_responses, 1);
        assert_eq!(got.evidence.omitted_rows, 0);
        assert!(got.links.is_empty());
    }
    let replies = vec![
        summary(json!({"A": {"count": 1}, "B": {"count": 2}})),
        (
            200,
            json!({"detail": [{}, {"pullRequests": [link(), {"url": 7}, null, {"url": ""}]}]}),
        ),
        (
            200,
            json!({"detail": [{"pullRequests": "SECRET"}, {"pullRequests": [link(), {}]}]}),
        ),
    ];
    let expected = block(Client::over(Replay::new(replies.clone())).code("10012")).unwrap();
    let got = read(replies);
    assert_eq!(got.links, expected);
    assert_eq!(got.links.len(), 3);
    assert_eq!(got.evidence.incomplete_responses, 2);
    assert_eq!(got.evidence.omitted_rows, 3);
    let effects = observed(&action(&site(), "ABC-12"), &Ok(got));
    assert_eq!(
        observations(&effects),
        [
            ("jira.sync.code.links", Some(3), Outcome::Partial, None),
            (
                "jira.sync.code.rows.omitted",
                Some(3),
                Outcome::Partial,
                None
            ),
            (
                "jira.sync.code.responses.incomplete",
                Some(2),
                Outcome::Partial,
                None
            ),
        ]
    );
}

#[test]
fn invalid_issue_ids_skip_all_requests_and_do_not_observe_empty_remote_results() {
    for id in ["", "12; SECRET", "-12", "１２", "12.0"] {
        let replay = Replay::new(vec![]);
        let client = Client::over(replay.clone());
        assert!(block(client.code(id)).unwrap().is_empty());
        let got = block(client.code_observed(id)).unwrap();
        assert!(replay.paths().is_empty());
        assert!(got.evidence.skipped_invalid_id);
        let effects = observed(&action(&site(), "ABC-12"), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Blocked)));
        assert_eq!(
            observations(&effects),
            [(
                "jira.sync.code.requests.skipped",
                Some(1),
                Outcome::Blocked,
                Some(Reason::InvalidInput)
            )]
        );
    }
}

#[test]
fn client_construction_still_precedes_the_invalid_id_guard() {
    let mut requested = site();
    requested.email.clear();
    let mut ticket = demo::sample().data["acme"].tickets["ABC-12"].clone();
    ticket.id = "SECRET_INVALID_ID".into();
    let Effect::SpawnAction { action, task } = crate::load_code(requested, ticket, Default::default(), Default::default()) else {
        panic!("development reads require a task-scoped action");
    };
    let message = block(task)
        .downcast::<JiraMsg>()
        .unwrap_or_else(|_| panic!("wrong result type"));
    let JiraMsg::Code {
        action: captured,
        result,
        ..
    } = *message
    else {
        panic!("wrong message")
    };
    assert_eq!(captured.subjects, action.subjects);
    assert!(
        result.is_err(),
        "missing client configuration wins before the ID guard, as before"
    );
    assert_eq!(
        observations(&observed(&captured, &result)),
        [(
            "jira.sync.code.errors",
            Some(1),
            Outcome::Failed,
            Some(Reason::Unspecified)
        ),]
    );
}

#[test]
fn errors_discard_earlier_progress_and_retries_keep_the_existing_requests() {
    for late in [false, true] {
        let mut replies = if late {
            vec![
                summary(json!({"A": {"count": 1}, "B": {"count": 1}})),
                detail(json!([link()])),
            ]
        } else {
            vec![]
        };
        replies.extend((0..3).map(|_| (503, json!({"errorMessages": ["SECRET_ERROR"]}))));
        let replay = Replay::new(replies);
        let result =
            block(Client::over(replay.clone()).code_observed("10012")).map_err(|e| e.to_string());
        assert!(result.is_err());
        assert_eq!(replay.paths().len(), if late { 5 } else { 3 });
        let effects = observed(&action(&site(), "ABC-12"), &result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Failed)));
        assert_eq!(
            observations(&effects),
            [(
                "jira.sync.code.errors",
                Some(1),
                Outcome::Failed,
                Some(Reason::Unspecified)
            )]
        );
    }
    let replay = Replay::new(vec![
        (429, json!({})),
        (503, json!({})),
        summary(json!({"A": {"count": 1}})),
        (429, json!({})),
        detail(json!([link()])),
    ]);
    let got = block(Client::over(replay.clone()).code_observed("10012")).unwrap();
    assert_eq!(got.links.len(), 1);
    assert_eq!(got.evidence, CodeEvidence::default());
    assert_eq!(replay.paths().len(), 5);
}

#[test]
fn captured_subject_is_only_the_validated_ticket_and_server() {
    let requested = site();
    let scope = action(&requested, "ABC-12");
    assert_eq!(scope.subjects[0].key.instance, "https://jira.example/");
    assert_eq!(scope.subjects[0].key.id, "ABC-12");
    assert_eq!(scope.subjects[0].role, SubjectRole::Primary);
    assert!(scope.subjects[0].url.is_none());
    assert!(!format!("{scope:?}").contains("SECRET"));
    let mut other = requested.clone();
    other.url = "https://other.example".into();
    assert_ne!(
        scope.subjects[0].key,
        action(&other, "ABC-12").subjects[0].key
    );
    for (url, ticket) in [
        ("https://[", "ABC-12"),
        ("file:///SECRET", "ABC-12"),
        ("https://jira.example", "SECRET bad key"),
    ] {
        other.url = url.into();
        let got = action(&other, ticket);
        assert!(got.subjects.is_empty());
        assert!(got.metadata.redacted);
        assert_eq!(got.metadata.omitted_subjects, 1);
        assert!(!format!("{got:?}").contains("SECRET"));
    }
}

#[test]
fn returned_evidence_is_not_added_to_the_provider_cache() {
    let mut plugin = demo::sample();
    plugin.cfg.views.clear();
    let got = read(vec![
        summary(json!({"A": {"count": 2}})),
        detail(json!([link(), {}])),
    ]);
    let links = got.links.clone();
    let descriptor = action(plugin.cfg.site("acme").unwrap(), "ABC-12");
    plugin
        .sync
        .code_in_flight
        .insert(crate::detail_key("acme", "ABC-12"));
    let effects = plugin.on_msg(Box::new(JiraMsg::Code {
        site: "acme".into(),
        key: "ABC-12".into(),
        updated: "SECRET_UPDATED".into(),
        action: descriptor,
        result: Ok(got),
    }));
    assert!(plugin.sync.code_in_flight.is_empty());
    assert_eq!(plugin.data["acme"].code["ABC-12"].links, links);
    assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
    assert!(matches!(effects.last(), Some(Effect::PersistCache)));
    let cache = plugin.snapshot().unwrap();
    let entry = cache["data"]["acme"]["code"]["ABC-12"].as_object().unwrap();
    assert_eq!(
        entry.keys().map(String::as_str).collect::<Vec<_>>(),
        ["at", "links", "updated"]
    );
    assert!(!cache.to_string().contains("omitted_rows"));
    assert!(!cache.to_string().contains("incomplete_responses"));
}

enum Begin {
    Start,
    Reconfigure,
}
struct Reads(JiraPlugin);

impl Plugin for Reads {
    fn id(&self) -> &'static str {
        "jira"
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
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "jira.test.write",
                        Origin::Plugin("jira".into()),
                    ))];
                    for index in 0..2 {
                        let mut requested = site();
                        if index == 1 {
                            requested.url = "https://other.example".into();
                        }
                        let result = if index == 1 {
                            Err("SECRET_ERROR".into())
                        } else {
                            Ok(read(vec![
                                summary(json!({"SECRET_APPLICATION": {"count": 1}})),
                                detail(json!([link()])),
                            ]))
                        };
                        effects.push(spawn(
                            action(&requested, "ABC-12"),
                            "SECRET_ALIAS".into(),
                            "ABC-12".into(),
                            "SECRET_UPDATED".into(),
                            async move { result },
                        ));
                    }
                    effects.push(Effect::Notify("SECRET_NOTICE".into()));
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    self.0.cfg.sites.clear();
                    self.0.cfg.views.clear();
                    vec![]
                }
            },
            Err(message) => self.0.on_msg(message),
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
fn reverse_order_completions_keep_captured_scope_and_export_no_payloads() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let effects = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let mut all = records(&effects);
    let mut tasks = Vec::new();
    for effect in effects {
        if let HostEffect::Tracked {
            action,
            semantic: Some(semantic),
            effect,
        } = effect
            && let HostEffect::Spawn { task, .. } = *effect
        {
            tasks.push((action, semantic, task));
        }
    }
    assert_eq!(tasks.len(), 2);
    assert_ne!(tasks[0].1, tasks[1].1);
    let write = all
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "jira.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
    let semantics: Vec<_> = tasks
        .iter()
        .map(|(_, semantic, _)| semantic.clone())
        .collect();
    assert!(
        semantics.iter().all(|id| terminal(&all, id).is_empty()),
        "dispatch is not read success"
    );
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for (index, (task_id, semantic, task)) in tasks.into_iter().enumerate().rev() {
        let message = block(task);
        let got = records(&host.update(Event::Tracked {
            action: task_id.clone(),
            semantic: Some(semantic.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        }));
        assert_eq!(
            terminal(&got, &semantic),
            [if index == 1 {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            }]
        );
        assert!(terminal(&got, &semantics[1 - index]).is_empty());
        assert!(terminal(&got, &write).is_empty());
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
            {
                assert_eq!(
                    parent.as_ref(),
                    Some(&task_id),
                    "observations do not capture later cache effects"
                );
                if descriptor.kind.as_str().starts_with("jira.sync.code.") {
                    assert_eq!(descriptor.subjects.len(), 1);
                    assert_eq!(
                        descriptor.subjects[0].key.instance,
                        if index == 1 {
                            "https://other.example/"
                        } else {
                            "https://jira.example/"
                        }
                    );
                }
            }
        }
        all.extend(got);
    }
    let dir =
        std::env::temp_dir().join(format!("radar-jira-code-history-{}", ActionId::new().as_str()));
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
    assert!(!exported.contains("elsewhere.example"));
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.sync.code.links"
                && event.metadata.count == Some(1))
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.sync.code.errors"
                && event.metadata.count == Some(1))
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}
