use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, MAX_TICKETS};
use crate::{JiraPlugin, demo, job};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@jira.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const JQL: &str = "summary ~ SECRET_QUERY ORDER BY updated DESC";

fn site() -> Site {
    let mut site = demo::sample().cfg.sites.remove(0);
    site.name = "SECRET_ALIAS".into();
    site.url = URL.into();
    site.email = "SECRET_EMAIL".into();
    site.token = "SECRET_TOKEN".into();
    site
}

fn issue(n: usize) -> Value {
    json!({
        "key": format!("ABC-{n}"),
        "fields": {
            "summary": "SECRET_TITLE",
            "description": "SECRET_BODY",
            "project": { "key": "ABC", "name": "SECRET_PROJECT" },
            "assignee": { "displayName": "SECRET_PERSON", "accountId": "SECRET_ACCOUNT" }
        }
    })
}

fn page(start: usize, len: usize, more: bool) -> (u16, Value) {
    (
        200,
        json!({
            "issues": (start..start + len).map(issue).collect::<Vec<_>>(),
            "isLast": !more,
            "nextPageToken": if more { Some("SECRET_CURSOR") } else { None }
        }),
    )
}

async fn read(client: Client) -> Result<Synced, Failed> {
    client
        .tickets_observed(JQL, &|_| None, &[])
        .await
        .map(|list| Synced { list, ..Default::default() })
        .map_err(|error| Failed::of(&error))
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
fn final_page_at_the_bound_is_complete_and_a_real_next_page_is_a_cutoff() {
    for more in [false, true] {
        let replay = Replay::new((0..4).map(|n| page(n * 100, 100, n < 3 || more)).collect());
        let got = block(read(Client::over(replay.clone()))).unwrap();
        assert_eq!(got.list.tickets.len(), MAX_TICKETS);
        assert_eq!(got.list.evidence.truncated, more);
        assert_eq!(got.list.evidence.omitted, 0);
        assert_eq!(got.list.evidence.incomplete_pages, 0);
        assert_eq!(replay.paths().len(), 4, "unchanged pagination bound");
        assert!(replay.paths()[1].ends_with("&nextPageToken=SECRET_CURSOR"));
        let outcome = if more {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        let effects = observed(&action(&site()), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(got) if got == outcome));
        let mut expected = vec![("jira.sync.tickets", Some(400), outcome, more)];
        if more {
            expected.push(("jira.sync.limits", Some(1), Outcome::Partial, true));
        }
        assert_eq!(observations(&effects), expected);
    }
}

#[test]
fn excess_rows_on_a_final_page_are_also_an_actual_cutoff() {
    let replay = Replay::new(vec![page(0, MAX_TICKETS + 1, false)]);
    let got = block(read(Client::over(replay.clone()))).unwrap();
    assert_eq!(replay.paths().len(), 1);
    assert_eq!(got.list.tickets.len(), MAX_TICKETS);
    assert!(got.list.evidence.truncated);
    assert_eq!(
        got.list.evidence.omitted, 0,
        "cutoff rows are not parse omissions"
    );
}

#[test]
fn parse_omissions_are_partial_without_changing_the_returned_ticket_rows() {
    let replies = vec![(
        200,
        json!({
            "issues": [issue(1), {"fields": {"summary": "SECRET_BAD_ROW"}}, issue(2)],
            "isLast": true
        }),
    )];
    let original =
        block(Client::over(Replay::new(replies.clone())).tickets(JQL, &|_| None, &[])).unwrap();
    let got = block(read(Client::over(Replay::new(replies)))).unwrap();
    assert_eq!(got.list.tickets, original);
    assert_eq!(got.list.evidence.omitted, 1);
    assert!(!got.list.evidence.truncated);
    let effects = observed(&action(&site()), &Ok(got));
    assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
    assert_eq!(
        observations(&effects),
        [
            ("jira.sync.tickets", Some(2), Outcome::Partial, false),
            ("jira.sync.issues.omitted", Some(1), Outcome::Partial, false),
        ]
    );
}

#[test]
fn known_empty_missing_list_and_incomplete_pagination_are_distinct() {
    let empty = block(read(Client::over(Replay::new(vec![page(0, 0, false)])))).unwrap();
    let effects = observed(&action(&site()), &Ok(empty));
    assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(
        observations(&effects),
        [("jira.sync.tickets", Some(0), Outcome::Succeeded, false)]
    );

    for body in [
        json!({"isLast": true}),
        json!({"issues": null, "isLast": true}),
        json!({"issues": "SECRET_NOT_LIST", "isLast": true}),
        json!({"issues": [issue(1)], "isLast": false}),
        json!({"issues": [], "isLast": false, "nextPageToken": "SECRET_CURSOR"}),
        json!({"issues": [], "nextPageToken": 12}),
        json!({"issues": [], "isLast": "SECRET_NOT_BOOL"}),
    ] {
        let replay = Replay::new(vec![(200, body)]);
        let got = block(read(Client::over(replay.clone()))).unwrap();
        assert_eq!(replay.paths().len(), 1, "no speculative extra request");
        assert_eq!(got.list.evidence.incomplete_pages, 1);
        assert_eq!(got.list.evidence.omitted, 0);
        assert!(!got.list.evidence.truncated);
        let effects = observed(&action(&site()), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
        assert_eq!(
            observations(&effects).last(),
            Some(&(
                "jira.sync.pages.incomplete",
                Some(1),
                Outcome::Partial,
                false,
            ))
        );
    }
}

#[test]
fn task_errors_do_not_invent_empty_results_or_prior_progress_and_retries_stay_intact() {
    for late in [false, true] {
        let mut replies = if late { vec![page(0, 1, true)] } else { vec![] };
        replies.extend((0..3).map(|_| (503, json!({"errorMessages": ["SECRET_ERROR"]}))));
        let replay = Replay::new(replies);
        let result = block(read(Client::over(replay.clone())));
        assert!(result.is_err());
        assert_eq!(replay.paths().len(), if late { 4 } else { 3 });
        let effects = observed(&action(&site()), &result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Failed)));
        assert_eq!(
            observations(&effects),
            [("jira.sync.errors", Some(1), Outcome::Failed, false)]
        );
    }
    let replay = Replay::new(vec![(429, json!({})), (503, json!({})), page(0, 0, false)]);
    let result = block(read(Client::over(replay.clone())));
    assert_eq!(replay.paths().len(), 3);
    assert!(matches!(
        observed(&action(&site()), &result)[0],
        Effect::Report(Outcome::Succeeded)
    ));
}

#[test]
fn blank_nonfinal_cursors_remain_incomplete_even_after_a_complete_later_page() {
    for cursor in ["", "  "] {
        let replay = Replay::new(vec![
            (
                200,
                json!({"issues": [issue(1)], "isLast": false, "nextPageToken": cursor}),
            ),
            page(2, 1, false),
        ]);
        let got = block(read(Client::over(replay.clone()))).unwrap();
        assert_eq!(
            replay.paths().len(),
            2,
            "preserve the existing request loop"
        );
        assert_eq!(got.list.tickets.len(), 2);
        assert_eq!(got.list.evidence.incomplete_pages, 1);
        assert!(!got.list.evidence.truncated);
        assert!(matches!(
            observed(&action(&site()), &Ok(got))[0],
            Effect::Report(Outcome::Partial)
        ));
    }
}

#[test]
fn unusable_continuation_at_the_exact_bound_does_not_invent_a_cutoff() {
    for cursor in [Value::Null, json!(""), json!("  "), json!(12)] {
        let replay = Replay::new(vec![(
            200,
            json!({
                "issues": (0..MAX_TICKETS).map(issue).collect::<Vec<_>>(),
                "isLast": false,
                "nextPageToken": cursor,
            }),
        )]);
        let got = block(read(Client::over(replay.clone()))).unwrap();
        assert_eq!(replay.paths().len(), 1);
        assert_eq!(got.list.tickets.len(), MAX_TICKETS);
        assert_eq!(got.list.evidence.incomplete_pages, 1);
        assert!(!got.list.evidence.truncated);
        let effects = observed(&action(&site()), &Ok(got));
        assert_eq!(
            observations(&effects),
            [
                ("jira.sync.tickets", Some(400), Outcome::Partial, false),
                (
                    "jira.sync.pages.incomplete",
                    Some(1),
                    Outcome::Partial,
                    false
                ),
            ]
        );
    }
}

#[test]
fn returned_task_keeps_original_scope_counts_before_cache_merge_and_preserves_cache_shape() {
    let mut plugin = demo::sample();
    let mut held = plugin.data["acme"].clone();
    held.me = None;
    held.found.clear();
    held.found.insert(
        "SECRET_OTHER_QUERY ORDER BY updated DESC".into(),
        held.tickets.keys().cloned().collect(),
    );
    let held_count = held.tickets.len();
    plugin.data.insert("SECRET_ALIAS".into(), held);
    plugin.cfg.views.push(crate::config::View {
        name: "SECRET_OTHER_VIEW".into(),
        when: vec![],
        jql: "SECRET_OTHER_QUERY ORDER BY updated DESC".into(),
        site: String::new(),
    });
    let requested = site();
    let effect = spawn(
        action(&requested),
        requested.name.clone(),
        JQL.into(),
        read(Client::over(Replay::new(vec![page(900, 2, false)]))),
    );
    let Effect::SpawnAction {
        action: captured,
        task,
    } = effect
    else {
        panic!("scoped task")
    };
    assert_eq!(captured.subjects[0].key.instance, "https://jira.example/");
    plugin.cfg.sites[0] = requested;
    plugin.cfg.views[0].jql = JQL.into();
    plugin.cfg.sites[0].url = "https://changed.example".into();
    plugin.sync.in_flight.insert(job("SECRET_ALIAS", JQL));
    let effects = plugin.on_msg(block(task));
    assert!(plugin.sync.in_flight.is_empty());
    assert_eq!(plugin.data["SECRET_ALIAS"].tickets.len(), held_count + 2);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistCache))
    );
    assert_eq!(
        observations(&effects),
        [("jira.sync.tickets", Some(2), Outcome::Succeeded, false)]
    );
    for effect in effects {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.instance, "https://jira.example/");
            assert_eq!(action.subjects[0].key.id, "site");
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
    let cache = plugin.snapshot().unwrap();
    let data = cache["data"]["SECRET_ALIAS"].as_object().unwrap();
    assert_eq!(
        data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["code", "found", "synced", "tickets"],
        "where each query stands is kept, so the next start asks only what changed"
    );
    assert!(!cache.to_string().contains("incomplete_pages"));
}

#[test]
fn invalid_scope_is_explicitly_omitted_and_distinct_servers_stay_distinct() {
    let mut requested = site();
    let first = action(&requested);
    requested.url = "https://other.example".into();
    let second = action(&requested);
    assert_eq!(first.subjects[0].key.id, second.subjects[0].key.id);
    assert_ne!(
        first.subjects[0].key.instance,
        second.subjects[0].key.instance
    );
    for url in [
        "https://[",
        "file:///private/SECRET",
        "https://token=SECRET@example.invalid",
    ] {
        requested.url = url.into();
        let captured = action(&requested);
        if url.contains("token=") {
            assert!(!format!("{captured:?}").contains("SECRET"));
        } else {
            assert!(captured.subjects.is_empty());
            assert!(captured.metadata.redacted);
            assert_eq!(captured.metadata.omitted_subjects, 1);
        }
    }
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
                    self.0.cfg.sites[0].url = URL.into();
                    self.0.cfg.views[0].jql = JQL.into();
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "jira.test.write",
                        Origin::Plugin("jira".into()),
                    ))];
                    effects.extend(self.0.refresh());
                    effects.push(Effect::Notify("SECRET_NOTICE".into()));
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    self.0.cfg.sites[0].url = "https://changed.example".into();
                    self.0.cfg.sites.clear();
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
fn out_of_order_results_close_only_their_scope_and_sqlite_exports_no_payloads() {
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
    assert_eq!(tasks.len(), 2);
    assert_ne!(tasks[0].1, tasks[1].1);
    let mut all = records(&effects);
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
                if descriptor.kind.as_str() == "jira.test.write" =>
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
            let mut got = block(read(Client::over(Replay::new(vec![page(1, 2, false)])))).unwrap();
            got.me = Some(Me {
                account_id: "SECRET_ACCOUNT".into(),
                name: "SECRET_PERSON".into(),
            });
            Ok(got)
        };
        let message = JiraMsg::Loaded {
            site: if index == 0 { "acme" } else { "beta" }.into(),
            jql: JQL.into(),
            action: scopes[index].clone(),
            result,
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
            {
                if descriptor.kind.as_str().starts_with("jira.sync.") {
                    assert_eq!(descriptor.subjects, scopes[index].subjects);
                    assert_eq!(parent.as_ref(), Some(&tasks[index].0));
                }
                if matches!(
                    descriptor.kind.as_str(),
                    "cache.save" | "ui.changed" | "ui.notice"
                ) {
                    assert_eq!(
                        parent.as_ref(),
                        Some(&tasks[index].0),
                        "summaries do not capture sibling effects"
                    );
                }
            }
        }
        all.extend(got);
    }
    let dir =
        std::env::temp_dir().join(format!("radar-jira-sync-history-{}", ActionId::new().as_str()));
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
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.sync.errors"
                && event.metadata.count == Some(1))
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind.as_str() == "jira.sync.tickets"
                && event.metadata.count == Some(2))
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}
