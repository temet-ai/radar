use super::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::client::recorded::block;
use crate::client::{Client, CommentEvidence, Method, Req, Resp, Sent, Transport};
use crate::{JiraPlugin, demo, detail_key, model::Comment, page};
use radar_core::Plugin;
use radar_core::journal::{Origin, SubjectRole};
use serde_json::{Value, json};

const SITE: &str = "acme";
const KEY: &str = "ABC-12";

enum Reply {
    Json(u16, Value),
    InvalidJson,
    Network,
    Pending,
}

struct Recorded {
    replies: Mutex<VecDeque<Reply>>,
    asked: Mutex<Vec<Req>>,
}

impl Recorded {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            asked: Mutex::default(),
        })
    }
    fn body(body: Value) -> Arc<Self> {
        Self::new(vec![Reply::Json(200, body)])
    }
    fn requests(&self) -> Vec<Req> {
        self.asked.lock().unwrap().clone()
    }
}
impl Transport for Recorded {
    fn send(&self, req: &Req) -> Sent<'_> {
        self.asked.lock().unwrap().push(req.clone());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("one recorded response per request");
        Box::pin(async move {
            match reply {
                Reply::Json(status, value) => Ok(Resp {
                    status,
                    retry_after: None,
                    rate: Default::default(),
                    body: value.to_string(),
                }),
                Reply::InvalidJson => Ok(Resp {
                    status: 200,
                    retry_after: None,
                    rate: Default::default(),
                    body: "SECRET_INVALID_JSON".into(),
                }),
                Reply::Network => Err("SECRET_NETWORK_ERROR".into()),
                Reply::Pending => std::future::pending().await,
            }
        })
    }
}

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
fn comment() -> Value {
    json!({"id": "SECRET_ID", "author": {"displayName": "SECRET_AUTHOR"}, "created": "SECRET_DATE", "body": "SECRET_BODY https://secret.example/SECRET_LINK"})
}
fn read(transport: Arc<Recorded>, start: usize) -> Result<CommentsRead, String> {
    block(Client::over(transport).comments_observed(KEY, start, COMMENTS_PAGE))
        .map_err(|e| format!("{e:#}"))
}
fn good() -> CommentsRead {
    CommentsRead {
        comments: vec![Comment::from_json(&comment())],
        total: 99,
        evidence: CommentEvidence::default(),
    }
}
fn observation(effects: &[Effect], kind: &str) -> Option<(u32, Outcome, bool, Option<Reason>)> {
    effects.iter().find_map(|effect| match effect {
        Effect::Observe { action, outcome } if action.kind.as_str() == kind => Some((
            action.metadata.count.unwrap(),
            *outcome,
            action.metadata.truncated,
            action.metadata.reason,
        )),
        _ => None,
    })
}
fn reported(effects: &[Effect]) -> Outcome {
    let Effect::Report(outcome) = effects[0] else {
        panic!("actual result must be observed first")
    };
    outcome
}
fn request(p: &mut JiraPlugin) -> Vec<Effect> {
    p.pages.last_mut().expect("open page").tab = 1;
    let end = radar_core::view::Intent::End {
        pane: radar_core::view::PaneId::Middle,
    };
    let radar_core::plugin::Outcome::Handled(fx) = page::intent(p, &end) else {
        panic!("comments scroll handled")
    };
    fx
}
fn installed(start: usize) -> JiraPlugin {
    let mut p = demo::sample();
    page::open(&mut p, SITE, KEY);
    let mut detail = crate::tests::detail();
    detail.comments = vec![Comment::from_json(&comment()); start];
    detail.comments_total = start + 100;
    crate::tests::complete_detail(&mut p, SITE, KEY, Ok(detail));
    p
}
fn owner(p: &JiraPlugin) -> page::More {
    p.details[&detail_key(SITE, KEY)].more.unwrap()
}
fn receive(
    p: &mut JiraPlugin,
    scope: &ActionDescriptor,
    read: page::More,
    result: Result<CommentsRead, String>,
) -> Vec<Effect> {
    p.on_msg(Box::new(JiraMsg::More {
        site: SITE.into(),
        key: KEY.into(),
        action: scope.clone(),
        read,
        result,
    }))
}

#[test]
fn one_page_preserves_request_wrapper_rows_and_total_without_inventing_inventory() {
    for (rows, total) in [
        (vec![], 0),
        (vec![], 100),
        (vec![comment(), Value::Null, json!({})], 100),
        (vec![Value::Null; 51], 100),
        (vec![Value::Null; 51], 1),
    ] {
        let body = json!({"comments": rows, "total": total});
        let plain = Recorded::body(body.clone());
        let expected = block(Client::over(plain.clone()).comments(KEY, 7, COMMENTS_PAGE)).unwrap();
        let recorded = Recorded::body(body);
        let got = read(recorded.clone(), 7).unwrap();
        assert_eq!(expected, (got.comments.clone(), got.total));
        assert_eq!(got.evidence, CommentEvidence::default());
        assert_eq!(
            got.comments.len(),
            rows.len(),
            "every permissive parser row retained"
        );
        assert_eq!(recorded.requests(), plain.requests());
        assert_eq!(
            recorded.requests(),
            [Req {
                method: Method::Get,
                path: "/rest/api/3/issue/ABC-12/comment?startAt=7&maxResults=50&orderBy=-created"
                    .into(),
                body: None
            }]
        );
        let fx = observed(&action(&site(), KEY, 7), &Ok(got));
        assert_eq!(reported(&fx), Outcome::Succeeded);
        assert_eq!(
            observation(&fx, "jira.page.comments.entries"),
            Some((rows.len() as u32, Outcome::Succeeded, false, None))
        );
        assert_eq!(
            observation(&fx, "jira.page.comments.limit.requested"),
            Some((50, Outcome::Succeeded, false, None))
        );
        assert_eq!(
            fx.len(),
            3,
            "older advertised rows and oversized replies add no gaps"
        );
    }
}

#[test]
fn missing_collections_and_defaulted_totals_retain_legacy_values_with_precise_gaps() {
    for body in [
        Value::Null,
        json!({}),
        json!({"comments": null, "total": 99}),
        json!({"comments": {}, "total": 99}),
        json!({"comments": []}),
        json!({"comments": [comment(), null, {}], "total": "SECRET_TOTAL"}),
        json!({"comments": [comment()], "total": -1}),
        json!({"comments": [comment()], "total": 1.5}),
    ] {
        let plain = Recorded::body(body.clone());
        let expected = block(Client::over(plain.clone()).comments(KEY, 9, COMMENTS_PAGE)).unwrap();
        let recorded = Recorded::body(body.clone());
        let got = read(recorded.clone(), 9).unwrap();
        assert_eq!(expected, (got.comments.clone(), got.total));
        assert_eq!(recorded.requests(), plain.requests());
        let collection_gap = !body.get("comments").is_some_and(Value::is_array);
        let total_gap = body.get("total").and_then(Value::as_u64).is_none();
        assert_eq!(
            got.evidence,
            CommentEvidence {
                incomplete_collection: collection_gap,
                total_defaulted: total_gap
            }
        );
        if total_gap {
            assert_eq!(got.total, 9 + got.comments.len());
        }
        let fx = observed(&action(&site(), KEY, 9), &Ok(got));
        assert_eq!(reported(&fx), Outcome::Partial);
        for (kind, present) in [
            ("collections.incomplete", collection_gap),
            ("total.defaulted", total_gap),
        ] {
            assert_eq!(
                observation(&fx, &format!("jira.page.comments.{kind}")),
                present.then_some((1, Outcome::Partial, false, None))
            );
        }
        assert!(fx.iter().all(|effect| !matches!(effect, Effect::Observe { action, .. } if action.metadata.truncated || action.kind.as_str().contains("omitted"))));
    }
}

#[test]
fn failures_record_one_logical_error_and_keep_request_retries_unchanged() {
    for (replies, attempts) in [
        (vec![Reply::Network], 1),
        (vec![Reply::InvalidJson], 1),
        (
            vec![Reply::Json(403, json!({"errorMessages": ["SECRET_ERROR"]}))],
            1,
        ),
        (
            vec![
                Reply::Json(503, json!({})),
                Reply::Json(429, json!({})),
                Reply::Json(503, json!({})),
            ],
            3,
        ),
    ] {
        let recorded = Recorded::new(replies);
        let result = read(recorded.clone(), 11);
        assert!(result.is_err());
        assert_eq!(recorded.requests().len(), attempts);
        assert!(
            recorded
                .requests()
                .windows(2)
                .all(|pair| pair[0] == pair[1])
        );
        let fx = observed(&action(&site(), KEY, 11), &result);
        assert_eq!(reported(&fx), Outcome::Failed);
        assert_eq!(fx.len(), 3);
        assert_eq!(
            observation(&fx, "jira.page.comments.errors"),
            Some((1, Outcome::Failed, false, Some(Reason::Unspecified)))
        );
        assert!(observation(&fx, "jira.page.comments.entries").is_none());
        assert_eq!(
            observation(&fx, "jira.page.comments.limit.requested")
                .unwrap()
                .0,
            50
        );
    }
}

#[test]
fn a_successful_retry_has_no_fabricated_error_or_http_attempt_counts() {
    let recorded = Recorded::new(vec![
        Reply::Json(429, json!({})),
        Reply::Json(503, json!({})),
        Reply::Json(200, json!({"comments": [comment()], "total": 80})),
    ]);
    let result = read(recorded.clone(), 4);
    assert_eq!(recorded.requests().len(), 3);
    let fx = observed(&action(&site(), KEY, 4), &result);
    assert_eq!(reported(&fx), Outcome::Succeeded);
    assert_eq!(fx.len(), 3);
}

#[test]
fn invalid_keys_do_not_issue_requests_or_leak_into_descriptors() {
    let recorded = Recorded::new(vec![]);
    let result =
        block(Client::over(recorded.clone()).comments_observed("SECRET bad key", 7, COMMENTS_PAGE))
            .map_err(|e| format!("{e:#}"));
    assert!(result.is_err());
    assert!(recorded.requests().is_empty());
    let scope = action(&site(), "SECRET bad key", 7);
    assert!(scope.subjects.is_empty());
    assert!(scope.metadata.redacted);
    assert_eq!(scope.metadata.omitted_subjects, 1);
    assert_eq!(reported(&observed(&scope, &result)), Outcome::Failed);
    assert!(!format!("{scope:?}").contains("SECRET"));
}

#[test]
fn action_captures_requested_offset_and_safe_server_ticket_not_alias_or_payload() {
    let mut requested = site();
    let scope = action(&requested, KEY, 17);
    requested.url = "https://replacement.example".into();
    assert_eq!(scope.kind.as_str(), "jira.page.comments");
    assert_eq!(scope.metadata.count, Some(17));
    assert_eq!(scope.subjects[0].key.instance, "https://jira.example/");
    assert_eq!(scope.subjects[0].key.id, KEY);
    assert_eq!(scope.subjects[0].role, SubjectRole::Primary);
    assert!(scope.subjects[0].url.is_none());
    assert!(!format!("{scope:?}").contains("SECRET"));
    for url in ["https://[", "file:///SECRET"] {
        requested.url = url.into();
        let invalid = action(&requested, KEY, 17);
        assert!(invalid.subjects.is_empty());
        assert!(invalid.metadata.redacted);
        assert_eq!(invalid.metadata.omitted_subjects, 1);
        assert_eq!(invalid.metadata.count, Some(17));
    }
}

#[test]
fn ordinal_overflow_is_omitted_while_result_counts_saturate_without_wrapping() {
    let scope = action(&site(), KEY, usize::MAX);
    assert_eq!(scope.metadata.count, u32::try_from(usize::MAX).ok());
    assert_eq!(scope.metadata.truncated, usize::BITS > 32);
    assert_eq!(scope.metadata.redacted, usize::BITS > 32);
    let fx = [count(
        &scope,
        "jira.page.comments.entries",
        usize::MAX,
        Outcome::Succeeded,
    )];
    assert_eq!(
        observation(&fx, "jira.page.comments.entries"),
        Some((u32::MAX, Outcome::Succeeded, usize::BITS > 32, None))
    );
    let skip = [skipped(&scope)];
    assert_eq!(
        observation(&skip, "jira.page.comments.application.skipped"),
        Some((1, Outcome::Blocked, false, None))
    );
}

#[test]
fn dispatch_captures_owner_scope_and_resolves_client_only_when_polled() {
    let mut p = installed(7);
    p.cfg.sites[0].email.clear();
    crate::write_context::reconcile(&mut p);
    crate::tests::complete_detail(&mut p, SITE, KEY, Ok(crate::tests::detail()));
    let fx = request(&mut p);
    let read = owner(&p);
    let [Effect::SpawnAction { action, .. }] = &fx[..] else {
        panic!("independent comments task")
    };
    let scope = action.clone();
    assert_eq!(scope.metadata.count, Some(read.start as u32));
    assert_eq!(read.start, 2);
    p.cfg.sites[0].url = "https://replacement.example".into();
    let Effect::SpawnAction { task, .. } = fx.into_iter().next().unwrap() else {
        unreachable!()
    };
    let msg = block(task);
    let JiraMsg::More {
        action,
        read: captured,
        result,
        ..
    } = *msg.downcast::<JiraMsg>().unwrap()
    else {
        panic!("comments reply")
    };
    assert_eq!(captured, read);
    assert_eq!(action.subjects, scope.subjects);
    assert_eq!(action.metadata, scope.metadata);
    assert!(result.as_ref().unwrap_err().contains("no email set"));
    let fx = observed(&action, &result);
    assert_eq!(reported(&fx), Outcome::Failed);
    assert_eq!(
        observation(&fx, "jira.page.comments.limit.requested")
            .unwrap()
            .0,
        50
    );
}

#[test]
fn no_op_paging_paths_stay_silent() {
    for condition in [
        "no-detail",
        "missing-value",
        "full-read",
        "busy",
        "complete",
        "missing-site",
    ] {
        let mut p = installed(2);
        let id = detail_key(SITE, KEY);
        match condition {
            "no-detail" => {
                p.details.remove(&id);
            }
            "missing-value" => p.details.get_mut(&id).unwrap().value = None,
            "full-read" => {
                page::fetch(&mut p, SITE, KEY, true);
            }
            "busy" => {
                assert_eq!(request(&mut p).len(), 1);
            }
            "complete" => {
                p.details
                    .get_mut(&id)
                    .unwrap()
                    .value
                    .as_mut()
                    .unwrap()
                    .comments_total = 2
            }
            _ => p.cfg.sites.clear(),
        }
        assert!(request(&mut p).is_empty(), "{condition}");
    }
}

#[test]
fn successful_partial_and_failed_pages_keep_existing_cache_application_shape() {
    for result in [
        Ok(good()),
        Ok(CommentsRead {
            comments: vec![],
            total: 2,
            evidence: CommentEvidence {
                incomplete_collection: true,
                total_defaulted: true,
            },
        }),
        Err("SECRET_ERROR".into()),
    ] {
        let mut observed_plugin = installed(2);
        let mut legacy_plugin = installed(2);
        request(&mut observed_plugin);
        request(&mut legacy_plugin);
        let read = owner(&observed_plugin);
        let scope = action(observed_plugin.cfg.site(SITE).unwrap(), KEY, read.start);
        let legacy = result
            .as_ref()
            .map(|got| (got.comments.clone(), got.total))
            .map_err(Clone::clone);
        let legacy_owner = owner(&legacy_plugin);
        assert!(page::on_more(
            &mut legacy_plugin,
            SITE,
            KEY,
            legacy_owner,
            legacy
        ));
        let fx = receive(&mut observed_plugin, &scope, read, result);
        assert!(
            fx.iter()
                .all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. }))
        );
        assert_eq!(
            format!("{:?}", observed_plugin.details),
            format!("{:?}", legacy_plugin.details)
        );
        assert_eq!(observed_plugin.snapshot(), legacy_plugin.snapshot());
        let cache = observed_plugin.snapshot().unwrap().to_string();
        for forbidden in [
            "CommentsRead",
            "CommentEvidence",
            "total_defaulted",
            "jira.page.comments",
            "\"request\"",
            "\"revision\"",
            "\"generation\"",
        ] {
            assert!(!cache.contains(forbidden), "{forbidden}");
        }
    }
}

#[test]
fn replayed_success_and_failure_report_actual_outcome_before_skip_without_mutation() {
    for failure in [false, true] {
        let mut p = installed(2);
        request(&mut p);
        let read = owner(&p);
        let scope = action(p.cfg.site(SITE).unwrap(), KEY, read.start);
        receive(&mut p, &scope, read, Ok(good()));
        request(&mut p);
        let before = format!("{:?}", p.details);
        let fx = receive(
            &mut p,
            &scope,
            read,
            if failure {
                Err("SECRET_ERROR".into())
            } else {
                Ok(good())
            },
        );
        assert_eq!(
            reported(&fx),
            if failure {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            }
        );
        let Some(Effect::Observe {
            action,
            outcome: Outcome::Blocked,
        }) = fx.last()
        else {
            panic!("separate local skip")
        };
        assert_eq!(
            action.kind.as_str(),
            "jira.page.comments.application.skipped"
        );
        assert_eq!(action.subjects, scope.subjects);
        assert_eq!(action.metadata.count, Some(1));
        assert_eq!(format!("{:?}", p.details), before);
    }
}

mod host_tests;
