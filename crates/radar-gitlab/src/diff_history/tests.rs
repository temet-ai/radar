use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, DiffEvidence, Method, Resp, parse_file_diff};
use crate::model::{DiffRefs, FileStat, MrDetail};
use crate::{GitlabPlugin, demo, pages};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

const PROJECT: &str = "acme/team-a/api";
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const HEAD: &str = "SECRET_CACHED_HEAD";

fn row() -> Value {
    json!({"old_path": "SECRET_OLD_PATH", "new_path": "SECRET_NEW_PATH", "diff": "SECRET_DIFF",
        "web_url": "https://elsewhere.example/SECRET_URL", "title": "SECRET_TITLE", "body": "SECRET_BODY"})
}

fn read(body: Value) -> DiffRead {
    block(Client::over(Replay::new(vec![(200, body)])).diffs_observed(PROJECT, 263, 2)).unwrap()
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

fn owner(plugin: &mut GitlabPlugin, key: &str, page: usize) -> crate::page_ownership::Read {
    let read = crate::page_ownership::next(plugin);
    plugin
        .diffs
        .get_mut(key)
        .expect("test dispatch cache")
        .requests
        .insert(page, read);
    read
}

#[test]
fn empty_short_full_and_oversized_pages_preserve_rows_without_inventing_a_cutoff() {
    for n in [0, 1, DIFF_PAGE, DIFF_PAGE + 1] {
        let rows = vec![row(); n];
        let plain = Replay::new(vec![(200, json!(rows))]);
        let observed_replay = Replay::new(vec![(200, json!(rows))]);
        let files = block(Client::over(plain.clone()).diffs(PROJECT, 263, 2)).unwrap();
        let got =
            block(Client::over(observed_replay.clone()).diffs_observed(PROJECT, 263, 2)).unwrap();
        assert_eq!(
            got.files,
            rows.iter().map(parse_file_diff).collect::<Vec<_>>()
        );
        assert_eq!(files, got.files);
        assert_eq!(
            got.files.len(),
            n,
            "duplicates and over-limit replies are not clipped"
        );
        assert_eq!(got.evidence, DiffEvidence::default());
        assert_eq!(plain.asked(), observed_replay.asked());
        let asked = observed_replay.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].method, Method::Get);
        assert_eq!(
            asked[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/263/diffs?page=2&per_page=20"
        );
        assert!(asked[0].body.is_none() && asked[0].range.is_none());
        let scope = action(URL, PROJECT, 263, 2);
        assert_eq!(
            scope.metadata.count,
            Some(2),
            "the request action records page, not returned rows"
        );
        let effects = observed(&scope, &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        assert_eq!(
            observations(&effects),
            [
                (
                    "gitlab.page.diffs.limit.requested",
                    Some(20),
                    Outcome::Succeeded,
                    false
                ),
                (
                    "gitlab.page.diffs.entries",
                    Some(n as u32),
                    Outcome::Succeeded,
                    false
                ),
            ]
        );
    }
}

#[test]
fn explicit_content_gaps_count_entries_once_and_empty_text_alone_is_complete() {
    let mut rows = vec![row(); 5];
    rows[0]["diff"] = json!("");
    rows[1]["too_large"] = json!(true);
    rows[2]["collapsed"] = json!(true);
    rows[3]["too_large"] = json!(true);
    rows[3]["collapsed"] = json!(true);
    rows[3].as_object_mut().unwrap().remove("diff");
    rows[4]["too_large"] = json!(false);
    rows[4]["collapsed"] = json!(false);
    let got = read(json!(rows));
    assert_eq!(
        got.files,
        rows.iter().map(parse_file_diff).collect::<Vec<_>>()
    );
    assert_eq!(
        got.evidence,
        DiffEvidence {
            withheld_rows: 3,
            ..Default::default()
        }
    );
    let effects = observed(&action(URL, PROJECT, 263, 2), &Ok(got));
    assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
    assert!(observations(&effects).contains(&(
        "gitlab.page.diffs.content.withheld",
        Some(3),
        Outcome::Partial,
        false
    )));
    let got = read(json!([{"old_path":"a", "new_path":"a", "diff":""}]));
    assert_eq!(got.evidence, DiffEvidence::default());
}

#[test]
fn malformed_rows_remain_returned_and_each_is_counted_once() {
    let rows = json!([null, false, 2, [], {},
        {"old_path":"a", "new_path":"b", "diff":4},
        {"old_path":"", "new_path":7, "diff":null},
        {"old_path":"a", "new_path":"b", "diff":"", "too_large":"true"}, row()]);
    let got = read(rows.clone());
    assert_eq!(
        got.files,
        rows.as_array()
            .unwrap()
            .iter()
            .map(parse_file_diff)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        got.evidence,
        DiffEvidence {
            incomplete_rows: 8,
            ..Default::default()
        }
    );
    let effects = observed(&action(URL, PROJECT, 263, 2), &Ok(got));
    assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
    assert!(observations(&effects).contains(&(
        "gitlab.page.diffs.rows.incomplete",
        Some(8),
        Outcome::Partial,
        false
    )));
    assert!(observations(&effects).contains(&(
        "gitlab.page.diffs.entries",
        Some(9),
        Outcome::Partial,
        false
    )));
}

#[test]
fn absent_optional_flags_are_accepted_but_present_null_is_incomplete() {
    let mut rows = vec![row(); 4];
    rows[1]["collapsed"] = json!(false);
    rows[2]["too_large"] = Value::Null;
    rows[3]["new_file"] = Value::Null;
    rows[3]["renamed_file"] = Value::Null;
    let got = read(json!(rows));
    assert_eq!(
        got.files,
        rows.iter().map(parse_file_diff).collect::<Vec<_>>()
    );
    assert_eq!(
        got.evidence,
        DiffEvidence {
            incomplete_rows: 2,
            ..Default::default()
        }
    );
}

#[test]
fn malformed_or_missing_list_is_partial_while_legacy_empty_result_is_unchanged() {
    for body in [
        b"SECRET_INVALID_JSON".to_vec(),
        vec![],
        b"null".to_vec(),
        b"{}".to_vec(),
        b"false".to_vec(),
    ] {
        let replay = Replay::raw(vec![Resp {
            status: 200,
            body,
            ..Default::default()
        }]);
        let got = block(Client::over(replay.clone()).diffs_observed(PROJECT, 263, 2)).unwrap();
        assert!(got.files.is_empty());
        assert_eq!(
            got.evidence,
            DiffEvidence {
                incomplete_response: true,
                ..Default::default()
            }
        );
        assert_eq!(replay.asked().len(), 1);
        let effects = observed(&action(URL, PROJECT, 263, 2), &Ok(got));
        assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
        assert!(observations(&effects).contains(&(
            "gitlab.page.diffs.responses.incomplete",
            Some(1),
            Outcome::Partial,
            false
        )));
    }
}

#[test]
fn returned_errors_have_no_inferred_rows_and_no_new_retries() {
    for status in [401, 403, 404, 429, 500] {
        let replay = Replay::new(vec![
            (status, json!({"message":"SECRET_ERROR"})),
            (200, json!([row()])),
        ]);
        let result = block(Client::over(replay.clone()).diffs_observed(PROJECT, 263, 2))
            .map_err(|e| format!("{e:#}"));
        assert!(result.is_err());
        assert_eq!(replay.asked().len(), 1);
        let effects = observed(&action(URL, PROJECT, 263, 2), &result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Failed)));
        assert_eq!(
            observations(&effects),
            [
                (
                    "gitlab.page.diffs.limit.requested",
                    Some(20),
                    Outcome::Succeeded,
                    false
                ),
                ("gitlab.page.diffs.errors", Some(1), Outcome::Failed, false),
            ]
        );
        assert!(!format!("{effects:?}").contains("SECRET"));
    }
    let result = block(Client::over(Replay::new(vec![])).diffs_observed(PROJECT, 263, 2))
        .map_err(|e| e.to_string());
    assert!(matches!(
        observed(&action(URL, PROJECT, 263, 2), &result)[0],
        Effect::Report(Outcome::Failed)
    ));
}

struct Tokens(std::sync::atomic::AtomicUsize);
impl crate::auth::Tokens for Tokens {
    fn token(&self) -> anyhow::Result<String> {
        Ok("SECRET_TOKEN".into())
    }
    fn renew(&self) -> anyhow::Result<bool> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(true)
    }
}

#[test]
fn auth_still_renews_only_once_and_keeps_the_requested_page() {
    for statuses in [[401, 200], [401, 401]] {
        let replay = Replay::new(statuses.into_iter().map(|s| (s, json!([row()]))).collect());
        let tokens = Tokens(std::sync::atomic::AtomicUsize::new(0));
        let result = block(crate::auth::with_token(&tokens, |_| {
            let client = Client::over(replay.clone());
            async move { client.diffs_observed(PROJECT, 263, 2).await }
        }));
        assert_eq!(result.is_ok(), statuses[1] == 200);
        assert_eq!(tokens.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let asked = replay.asked();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0], asked[1]);
    }
}

#[test]
fn canonical_requested_identity_has_no_location_or_result_subject() {
    let scope = action(URL, PROJECT, 263, 2);
    let subject = &scope.subjects[0];
    assert_eq!(subject.key.instance, "https://git.example/");
    assert_eq!(subject.key.kind, "merge-request");
    assert_eq!(subject.key.id, "acme/team-a/api!263");
    assert_eq!(subject.role, SubjectRole::Primary);
    assert!(subject.url.is_none() && scope.metadata.url.is_none());
    let mut plugin = demo::sample();
    plugin.cfg.url = URL.into();
    let owner = plugin
        .journal_subject(
            &radar_core::Subject::new(join::MERGE_REQUEST, "acme/team-a/api!263", "SECRET_TITLE")
                .with_url("https://git.example/acme/team-a/api/-/merge_requests/263"),
        )
        .unwrap();
    assert_eq!(subject.key, owner.key);
    for (url, project, iid) in [
        ("not a URL", PROJECT, 263),
        ("file:///secret", PROJECT, 263),
        ("https://git.example/password=SECRET", PROJECT, 263),
        (URL, "acme/private?token=SECRET", 263),
        (URL, "acme/../api", 263),
        (URL, "acme/./api", 263),
        (URL, PROJECT, 0),
    ] {
        let scope = action(url, project, iid, 2);
        assert!(scope.subjects.is_empty() && scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
    }
    for effect in observed(&scope, &Ok(read(json!([row()])))) {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects, scope.subjects);
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
}

#[test]
fn numeric_page_is_the_raw_request_parameter_and_overflow_is_not_a_different_page() {
    let replay = Replay::new(vec![(200, json!([]))]);
    block(Client::over(replay.clone()).diffs_observed(PROJECT, 263, 0)).unwrap();
    assert!(replay.asked()[0].path.contains("page=0&per_page=20"));
    assert_eq!(action(URL, PROJECT, 263, 0).metadata.count, Some(0));
    if usize::BITS > 32 {
        let scope = action(URL, PROJECT, 263, usize::MAX);
        assert_eq!(scope.metadata.count, None);
        assert!(scope.metadata.truncated && scope.metadata.redacted);
    }
}

#[test]
fn actual_task_returns_original_scope_after_reconfiguration() {
    let mut plugin = demo::sample();
    let key = pages::mr_key(PROJECT, 263);
    plugin.diffs.entry(key.clone()).or_default().head = HEAD.into();
    let client = Client::over(Replay::new(vec![(200, json!([row()]))]));
    let effect = spawn(
        action(URL, PROJECT, 263, 2),
        key.clone(),
        owner(&mut plugin, &key, 2),
        HEAD.into(),
        2,
        async move {
            client
                .diffs_observed(PROJECT, 263, 2)
                .await
                .map_err(|e| e.to_string())
        },
    );
    let Effect::SpawnAction { task, .. } = effect else {
        panic!("scoped read")
    };
    plugin.cfg.url = "https://changed.example".into();
    let effects = plugin.on_msg(block(task));
    assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(plugin.diffs[&key].files.len(), 1);
    assert_eq!(plugin.diffs[&key].ended, Some(2));
    for effect in &effects {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.instance, "https://git.example/");
        }
    }
    let cache = plugin.snapshot().unwrap().to_string();
    for transient in [
        "incomplete_response",
        "incomplete_rows",
        "withheld_rows",
        "gitlab.page.diffs",
    ] {
        assert!(!cache.contains(transient));
    }
}

fn page_plugin() -> GitlabPlugin {
    let mut plugin = demo::sample();
    plugin.cfg.url = URL.into();
    let mr = pages::find_mr(&plugin, PROJECT, 263).unwrap().clone();
    plugin.mr_pages.insert(
        pages::mr_key(PROJECT, 263),
        pages::Fetched {
            value: Some((
                mr,
                MrDetail {
                    files: (0..DIFF_PAGE + 1)
                        .map(|n| FileStat {
                            path: format!("SECRET_FILE_{n}"),
                            additions: 0,
                            deletions: 0,
                        })
                        .collect(),
                    refs: Some(DiffRefs {
                        head: HEAD.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            at: plugin.now,
            ..Default::default()
        },
    );
    pages::push(
        &mut plugin,
        pages::Kind::Mr {
            project: PROJECT.into(),
            iid: 263,
        },
    );
    plugin.pages.last_mut().unwrap().tab = 1;
    plugin
}

enum Begin {
    Start,
    Reconfigure,
}
type Captured = std::sync::Arc<
    std::sync::Mutex<std::collections::BTreeMap<(String, usize), crate::page_ownership::Read>>,
>;
struct Reads(GitlabPlugin, Captured);
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
        let effects = match message.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Start => {
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "gitlab.test.write",
                        Origin::Plugin("gitlab".into()),
                    ))];
                    effects.extend(pages::changes::want(&mut self.0));
                    self.0
                        .diffs
                        .entry(pages::mr_key(PROJECT, 264))
                        .or_default()
                        .head = HEAD.into();
                    effects.push(spawn(
                        action(URL, PROJECT, 264, 3),
                        pages::mr_key(PROJECT, 264),
                        owner(&mut self.0, &pages::mr_key(PROJECT, 264), 3),
                        HEAD.into(),
                        3,
                        async { Err("SECRET_ERROR".into()) },
                    ));
                    effects.push(Effect::Notify("SECRET_NOTICE".into()));
                    effects.push(Effect::Report(Outcome::Succeeded));
                    effects
                }
                Begin::Reconfigure => {
                    self.0.cfg.url = "https://changed.example".into();
                    self.0
                        .diffs
                        .get_mut(&pages::mr_key(PROJECT, 264))
                        .unwrap()
                        .head = "SECRET_NEW_HEAD".into();
                    vec![]
                }
            },
            Err(message) => self.0.on_msg(message),
        };
        let mut captured = self.1.lock().unwrap();
        for (key, diffs) in &self.0.diffs {
            for (page, read) in &diffs.requests {
                captured.insert((key.clone(), *page), *read);
            }
        }
        effects
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

fn tasks(effects: &[HostEffect]) -> Vec<(ActionId, ActionId)> {
    effects
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

fn descriptor(records: &[JournalRecord], target: &ActionId) -> ActionDescriptor {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin { id, descriptor, .. } if id == target => Some(descriptor.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn host_isolates_write_out_of_order_stale_and_next_page_results_and_exports_no_payloads() {
    let captured = Captured::default();
    let mut host = Host::new(vec![Box::new(Reads(page_plugin(), captured.clone()))], 1).unwrap();
    host.enable_journal();
    let initial = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Start),
    });
    let pending = tasks(&initial);
    assert_eq!(pending.len(), 2);
    let mut all = records(&initial);
    let scopes: Vec<_> = pending
        .iter()
        .map(|(_, semantic)| descriptor(&all, semantic))
        .collect();
    assert_eq!(scopes[0].metadata.count, Some(1));
    assert_eq!(scopes[1].metadata.count, Some(3));
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
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reconfigure),
    })));
    for index in [1, 0] {
        let result = if index == 1 {
            Err("SECRET_ERROR".into())
        } else {
            Ok(read(json!(vec![row(); DIFF_PAGE])))
        };
        let read = captured.lock().unwrap()[&(
            pages::mr_key(PROJECT, 263 + index as u64),
            if index == 1 { 3 } else { 1 },
        )];
        let effects = host.update(Event::Tracked {
            action: pending[index].0.clone(),
            semantic: Some(pending[index].1.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: Box::new(GlMsg::Diffs {
                    key: pages::mr_key(PROJECT, 263 + index as u64),
                    read,
                    head: HEAD.into(),
                    page: if index == 1 { 3 } else { 1 },
                    action: scopes[index].clone(),
                    result,
                }),
            }),
        });
        let got = records(&effects);
        assert_eq!(
            terminal(&got, &pending[index].1),
            [if index == 1 {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            }]
        );
        assert!(terminal(&got, &pending[1 - index].1).is_empty());
        assert!(terminal(&got, &write).is_empty());
        let stale = got.iter().any(|record| matches!(record, JournalRecord::Begin { descriptor, .. } if descriptor.kind.as_str() == "gitlab.page.diffs.apply.skipped.stale"));
        assert_eq!(stale, index == 1);
        if index == 0 {
            let next = tasks(&effects);
            assert_eq!(
                next.len(),
                1,
                "unchanged follow-on asks page two for the still-missing selected file"
            );
            assert_ne!(next[0].1, pending[0].1);
            let scope = descriptor(&got, &next[0].1);
            assert_eq!(scope.metadata.count, Some(2));
            assert_eq!(
                scope.subjects[0].key.instance, "https://changed.example/",
                "new request captures its own current configuration"
            );
            assert!(terminal(&got, &next[0].1).is_empty());
            // Leave the next task unreturned. Session cleanup must keep its
            // outcome unknown, without borrowing the preceding page's rows.
        }
        for record in &got {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && descriptor.kind.as_str().starts_with("gitlab.page.diffs.")
            {
                assert_eq!(parent.as_ref(), Some(&pending[index].0));
                assert_eq!(descriptor.subjects[0].key.instance, "https://git.example/");
            }
        }
        all.extend(got);
    }
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-diff-history-{}",
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
    journal.finish_session(&session).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let exported = serde_json::to_string(&events).unwrap();
    assert!(!exported.contains("SECRET"), "{exported}");
    assert!(events.iter().any(
        |event| event.kind.as_str() == "gitlab.page.diffs" && event.outcome == Outcome::Unknown
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind.as_str() == "gitlab.page.diffs.entries"
                && event.outcome.is_terminal())
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.action_id == write && event.outcome.is_terminal())
            .map(|event| event.outcome)
            .collect::<Vec<_>>(),
        [Outcome::Succeeded]
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stale_success_still_records_original_rows_before_skipping_application() {
    let mut plugin = demo::sample();
    let key = pages::mr_key(PROJECT, 263);
    plugin.diffs.entry(key.clone()).or_default().head = "SECRET_NEW_HEAD".into();
    let read_owner = owner(&mut plugin, &key, 2);
    let effects = plugin.on_msg(Box::new(GlMsg::Diffs {
        key: key.clone(),
        read: read_owner,
        head: HEAD.into(),
        page: 2,
        action: action(URL, PROJECT, 263, 2),
        result: Ok(read(json!([row()]))),
    }));
    assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
    assert!(plugin.diffs[&key].files.is_empty());
    assert_eq!(
        observations(&effects).last().unwrap().0,
        "gitlab.page.diffs.apply.skipped.stale"
    );
    assert!(observations(&effects).contains(&(
        "gitlab.page.diffs.entries",
        Some(1),
        Outcome::Succeeded,
        false
    )));
}

#[test]
fn same_mr_and_page_on_two_servers_keep_original_scopes_across_head_ordering() {
    let mut plugin = demo::sample();
    let key = pages::mr_key(PROJECT, 263);
    let new_head = "SECRET_NEW_HEAD";
    plugin.cfg.url = "https://second.example".into();
    plugin.diffs.entry(key.clone()).or_default().head = new_head.into();
    // Completion order is newer, then older. Both requested precisely MR/page 2.
    for (url, head, path) in [
        ("https://second.example", new_head, "SECRET_NEW_FILE"),
        ("https://first.example", HEAD, "SECRET_OLD_FILE"),
    ] {
        let scope = action(url, PROJECT, 263, 2);
        let mut payload = row();
        payload["new_path"] = json!(path);
        let client = Client::over(Replay::new(vec![(200, json!([payload]))]));
        let read_owner = owner(&mut plugin, &key, 2);
        let effect = spawn(
            scope.clone(),
            key.clone(),
            read_owner,
            head.into(),
            2,
            async move {
                client
                    .diffs_observed(PROJECT, 263, 2)
                    .await
                    .map_err(|e| e.to_string())
            },
        );
        let Effect::SpawnAction { action, task } = effect else {
            panic!("scoped task")
        };
        assert_eq!(action.subjects, scope.subjects);
        assert_eq!(action.metadata.count, Some(2));
        let effects = plugin.on_msg(block(task));
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        let stale = head != new_head;
        assert_eq!(
            observations(&effects)
                .iter()
                .any(|o| o.0 == "gitlab.page.diffs.apply.skipped.stale"),
            stale
        );
        for effect in effects {
            if let Effect::Observe { action, .. } = effect {
                assert_eq!(action.subjects, scope.subjects);
            }
        }
        assert_eq!(plugin.diffs[&key].head, new_head);
        assert_eq!(
            plugin.diffs[&key]
                .files
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["SECRET_NEW_FILE"]
        );
    }
}
