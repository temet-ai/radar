use super::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::client::recorded::block;
use crate::client::{Client, DetailEvidence, Method, Req, Resp, Sent, Transport};
use crate::{JiraPlugin, demo, detail_key, model, page};
use radar_core::Plugin;
use radar_core::view::{Chord, Key};
use serde_json::{Value, json};

const ISSUE: &str = "/rest/api/3/issue/ABC-12";
const CHILDREN: &str = "/rest/api/3/search/jql";
const WEB: &str = "/rest/api/3/issue/ABC-12/remotelink";
const COMMENTS: &str = "/rest/api/3/issue/ABC-12/comment";
const SUMMARY: &str = "/rest/dev-status/latest/issue/summary";
const CODE: &str = "/rest/dev-status/latest/issue/detail";

enum Reply {
    Response(u16, Value),
    Network,
    Pending,
}

/// Route by endpoint rather than poll order: joined reads can interleave retries.
struct Routed {
    replies: Mutex<BTreeMap<&'static str, VecDeque<Reply>>>,
    asked: Mutex<Vec<Req>>,
}

impl Routed {
    fn complete() -> Self {
        Self {
            replies: Mutex::new(BTreeMap::from([
                (
                    ISSUE,
                    VecDeque::from([Reply::Response(200, issue("ABC-12"))]),
                ),
                (
                    CHILDREN,
                    VecDeque::from([Reply::Response(200, json!({"issues": [], "isLast": true}))]),
                ),
                (WEB, VecDeque::from([Reply::Response(200, json!([]))])),
                (
                    COMMENTS,
                    VecDeque::from([Reply::Response(200, json!({"comments": [], "total": 0}))]),
                ),
                (
                    SUMMARY,
                    VecDeque::from([Reply::Response(200, summary(json!({})))]),
                ),
            ])),
            asked: Mutex::default(),
        }
    }

    fn with(self, path: &'static str, replies: Vec<Reply>) -> Self {
        self.replies.lock().unwrap().insert(path, replies.into());
        self
    }

    fn body(self, path: &'static str, body: Value) -> Self {
        self.with(path, vec![Reply::Response(200, body)])
    }

    fn paths(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|req| req.path.clone())
            .collect()
    }

    fn count(&self, path: &str) -> usize {
        self.paths()
            .iter()
            .filter(|p| p.split('?').next() == Some(path))
            .count()
    }
}

impl Transport for Routed {
    fn send(&self, req: &Req) -> Sent<'_> {
        self.asked.lock().unwrap().push(req.clone());
        let endpoint = req.path.split('?').next().unwrap();
        let reply = self
            .replies
            .lock()
            .unwrap()
            .get_mut(endpoint)
            .and_then(VecDeque::pop_front);
        Box::pin(async move {
            match reply {
                Some(Reply::Response(status, body)) => Ok(Resp {
                    status,
                    retry_after: None,
                    rate: Default::default(),
                    body: body.to_string(),
                }),
                Some(Reply::Network) => Err("SECRET_NETWORK_ERROR".into()),
                Some(Reply::Pending) => std::future::pending().await,
                None => panic!("no recorded reply for dispatched request"),
            }
        })
    }
}

fn issue(key: &str) -> Value {
    json!({"id": "10012", "key": key, "fields": {
        "summary": "SECRET_TITLE", "project": {"key": "ABC"},
        "updated": "SECRET_UPDATED", "description": "SECRET_DESCRIPTION"
    }})
}

fn comment() -> Value {
    json!({"author": {"displayName": "SECRET_AUTHOR"}, "created": "SECRET_DATE", "body": "SECRET_COMMENT"})
}

fn summary(hosts: Value) -> Value {
    json!({"summary": {"pullrequest": {"byInstanceType": hosts}}})
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

fn read(transport: Routed) -> (Result<Box<DetailRead>, String>, Arc<Routed>) {
    let transport = Arc::new(transport);
    let result = block(Client::over(transport.clone()).detail_observed("ABC-12", None))
        .map(Box::new)
        .map_err(|e| format!("{e:#}"));
    (result, transport)
}

fn effects(result: &Result<Box<DetailRead>, String>) -> Vec<Effect> {
    observed(&action(&site(), "ABC-12"), result)
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
        panic!("report comes before cache application")
    };
    outcome
}

#[test]
fn explicit_empty_components_are_successful_returned_zeroes() {
    let (result, transport) = read(Routed::complete());
    let got = result.as_ref().unwrap();
    assert_eq!(got.evidence, DetailEvidence::default());
    assert_eq!(got.detail.comments_total, 0);
    assert!(got.detail.links.is_empty());
    let effects = effects(&result);
    assert_eq!(reported(&effects), Outcome::Succeeded);
    for kind in [
        "parents",
        "children",
        "issue.links",
        "web.links",
        "comments",
        "code.links",
    ] {
        assert_eq!(
            observation(&effects, &format!("jira.page.detail.{kind}")),
            Some((0, Outcome::Succeeded, false, None))
        );
    }
    assert_eq!(transport.paths().len(), 5);
    assert!(
        transport
            .asked
            .lock()
            .unwrap()
            .iter()
            .all(|req| req.method == Method::Get && req.body.is_none())
    );
}

fn rich() -> Routed {
    let mut main = issue("SECRET-42");
    main["fields"]["parent"] = json!({"key": "ABC-1", "fields": {"summary": "SECRET_PARENT"}});
    main["fields"]["customfield_10050"] = json!("SECRET_CRITERIA");
    main["fields"]["customfield_10020"] = json!([{"name": "SECRET_SPRINT", "state": "active"}]);
    main["names"] = json!({"customfield_10020": "Sprint"});
    main["fields"]["issuelinks"] =
        json!([{"outwardIssue": {"key": "ABC-2"}, "type": {"outward": "SECRET_RELATION"}}]);
    main["fields"]["attachment"] = json!([{"filename": "SECRET_IMAGE.png", "mimeType": "image/png", "content": "https://attachment.example/SECRET_ATTACHMENT"}]);
    let media = json!({"body": {"type": "doc", "content": [{"type": "mediaSingle", "content": [{"type": "media", "attrs": {"alt": "SECRET_IMAGE.png"}}]}]}});
    Routed::complete().body(ISSUE, main)
        .body(CHILDREN, json!({"issues": [issue("ABC-13"), issue("ABC-13")], "isLast": true}))
        .body(WEB, json!([{"object": {"url": "https://elsewhere.example/SECRET_INVITE?token=SECRET_TOKEN", "title": "SECRET_WEB_TITLE"}}, {"object": {"url": ""}}]))
        .body(COMMENTS, json!({"comments": [comment(), media], "total": 51}))
        .body(SUMMARY, summary(json!({"SECRET_APPLICATION": {"count": 3}})))
        .body(CODE, json!({"detail": [{"pullRequests": [{"url": "not validated SECRET_CODE_URL", "name": "SECRET_CODE_TITLE"}, {"url": ""}]}]}))
}

#[test]
fn public_detail_wrapper_keeps_exact_requests_link_order_and_permissive_values() {
    let old = Arc::new(rich());
    let expected =
        block(Client::over(old.clone()).detail("ABC-12", Some("customfield_10050".into())))
            .unwrap();
    let new = Arc::new(rich());
    let got = block(
        Client::over(new.clone()).detail_observed("ABC-12", Some("customfield_10050".into())),
    )
    .unwrap();
    assert_eq!(expected, got.detail);
    assert_eq!(old.paths(), new.paths());
    let paths = new.paths();
    assert_eq!(
        paths[0],
        "/rest/api/3/issue/ABC-12?fields=*all&expand=names"
    );
    let fields = model::list_fields(&["customfield_10050".into()]).join(",");
    assert_eq!(
        paths[1],
        format!(
            "{CHILDREN}?jql=parent%20%3D%20ABC-12%20ORDER%20BY%20created%20ASC&fields={}&maxResults=100",
            crate::client::enc(&fields)
        )
    );
    assert_eq!(
        &paths[2..],
        [
            WEB,
            &format!("{COMMENTS}?startAt=0&maxResults=50&orderBy=-created"),
            &format!("{SUMMARY}?issueId=10012"),
            &format!(
                "{CODE}?issueId=10012&applicationType=SECRET_APPLICATION&dataType=pullrequest"
            )
        ]
    );
    assert_eq!(
        got.detail
            .links
            .iter()
            .map(|l| l.relation.as_str())
            .collect::<Vec<_>>(),
        [
            "parent",
            "child",
            "child",
            "SECRET_RELATION",
            "web link",
            "web link"
        ]
    );
    assert_eq!(
        got.detail.ticket.key, "SECRET-42",
        "returned identity is not silently validated or replaced"
    );
    assert_eq!(got.detail.sprint.as_deref(), Some("SECRET_SPRINT"));
    assert_eq!(
        radar_core::ui::rich::plain(&got.detail.ticket.ac, 80),
        "SECRET_CRITERIA"
    );
    assert!(
        format!("{:?}", got.detail.comments[1].body)
            .contains("https://attachment.example/SECRET_ATTACHMENT")
    );
    assert_eq!(got.detail.code.len(), 2);
    assert_eq!(
        got.evidence,
        DetailEvidence {
            children_count: 2,
            issue_link_count: 1,
            web_link_count: 2,
            ..Default::default()
        }
    );
    assert_eq!(reported(&effects(&Ok(Box::new(got)))), Outcome::Succeeded);
}

#[test]
fn optional_read_failures_are_partial_without_invented_successful_zeroes() {
    for (path, error_kind, count_kind) in [
        (CHILDREN, "children.errors", "children"),
        (WEB, "web.errors", "web.links"),
        (SUMMARY, "code.errors", "code.links"),
    ] {
        let (result, transport) = read(Routed::complete().with(
            path,
            vec![Reply::Response(
                403,
                json!({"errorMessages": ["SECRET_ERROR"]}),
            )],
        ));
        assert!(result.is_ok());
        assert_eq!(transport.count(path), 1);
        let fx = effects(&result);
        assert_eq!(reported(&fx), Outcome::Partial);
        assert_eq!(
            observation(&fx, &format!("jira.page.detail.{error_kind}")),
            Some((1, Outcome::Failed, false, Some(Reason::Unspecified)))
        );
        assert!(observation(&fx, &format!("jira.page.detail.{count_kind}")).is_none());
        assert_eq!(
            observation(&fx, "jira.page.detail.tickets"),
            Some((1, Outcome::Succeeded, false, None))
        );
        assert_eq!(
            transport.count(SUMMARY),
            1,
            "optional joined failures do not prevent code"
        );
    }
}

#[test]
fn fatal_issue_or_comment_errors_preserve_no_result_progress_and_comments_prevent_code() {
    for path in [ISSUE, COMMENTS] {
        let (result, transport) = read(Routed::complete().with(path, vec![Reply::Network]));
        assert!(result.is_err());
        assert_eq!(transport.count(SUMMARY), 0);
        for sibling in [CHILDREN, WEB, COMMENTS] {
            assert_eq!(transport.count(sibling), usize::from(path == COMMENTS));
        }
        let fx = effects(&result);
        assert_eq!(fx.len(), 2);
        assert_eq!(reported(&fx), Outcome::Failed);
        assert_eq!(
            observation(&fx, "jira.page.detail.errors"),
            Some((1, Outcome::Failed, false, Some(Reason::Unspecified)))
        );
    }
}

#[test]
fn failed_required_ticket_parser_does_not_dispatch_joined_reads() {
    for body in [json!({}), json!({"key": 7}), Value::Null] {
        let (result, transport) = read(Routed::complete().body(ISSUE, body));
        assert!(result.unwrap_err().contains("has no ticket"));
        assert_eq!(transport.paths().len(), 1);
    }
}

#[test]
fn fatal_comments_wait_for_joined_siblings_but_discard_their_evidence() {
    let (result, transport) = read(
        Routed::complete()
            .body(
                CHILDREN,
                json!({"issues": [issue("ABC-13"), {}], "isLast": true}),
            )
            .with(
                WEB,
                vec![
                    Reply::Response(503, json!({})),
                    Reply::Response(429, json!({})),
                    Reply::Response(200, json!([{"object": {"url": "SECRET_URL"}}])),
                ],
            )
            .with(COMMENTS, vec![Reply::Response(403, json!({}))]),
    );
    assert!(result.is_err());
    assert_eq!(
        transport.count(WEB),
        3,
        "join waits for the optional sibling's existing retries"
    );
    assert_eq!(transport.count(SUMMARY), 0);
    assert_eq!(
        effects(&result).len(),
        2,
        "no earlier successful or partial progress was returned"
    );
}

#[test]
fn optional_retries_are_unchanged_and_count_only_final_logical_failure() {
    let (result, transport) = read(
        Routed::complete()
            .with(
                CHILDREN,
                vec![
                    Reply::Response(429, json!({})),
                    Reply::Response(503, json!({})),
                    Reply::Response(200, json!({"issues": [], "isLast": true})),
                ],
            )
            .with(
                WEB,
                (0..3)
                    .map(|_| Reply::Response(503, json!({"errorMessages": ["SECRET_ERROR"]})))
                    .collect(),
            ),
    );
    assert_eq!(transport.count(CHILDREN), 3);
    assert_eq!(transport.count(WEB), 3);
    let fx = effects(&result);
    assert_eq!(reported(&fx), Outcome::Partial);
    assert_eq!(
        observation(&fx, "jira.page.detail.children"),
        Some((0, Outcome::Succeeded, false, None))
    );
    assert_eq!(
        observation(&fx, "jira.page.detail.web.errors").unwrap().0,
        1
    );
}

#[test]
fn children_cutoff_distinguishes_exact_final_bound_extra_rows_and_unread_continuation() {
    for (n, last, truncated) in [
        (399, true, false),
        (400, true, false),
        (401, true, true),
        (400, false, true),
    ] {
        let (result, transport) = read(Routed::complete().body(CHILDREN, json!({"issues": vec![issue("ABC-13"); n], "isLast": last, "nextPageToken": "SECRET_CURSOR"})));
        let got = result.as_ref().unwrap();
        assert_eq!(got.evidence.children_count, n.min(400));
        assert_eq!(got.evidence.children.truncated, truncated);
        assert_eq!(transport.count(CHILDREN), 1);
        let fx = effects(&result);
        assert_eq!(reported(&fx), outcome(truncated));
        assert_eq!(
            observation(&fx, "jira.page.detail.children").unwrap().2,
            truncated
        );
        assert_eq!(
            observation(&fx, "jira.page.detail.children.limits").is_some(),
            truncated
        );
    }
}

#[test]
fn children_keep_existing_continuation_bytes_and_known_parser_omissions() {
    let (result, transport) = read(Routed::complete().with(
        CHILDREN,
        vec![
            Reply::Response(
                200,
                json!({"issues": [issue("ABC-13")], "isLast": false, "nextPageToken": "SECRET +/"}),
            ),
            Reply::Response(
                200,
                json!({"issues": [{}, {"key": 1}, {"key": ""}], "isLast": true}),
            ),
        ],
    ));
    assert!(
        transport
            .paths()
            .iter()
            .any(|p| p.ends_with("&nextPageToken=SECRET%20%2B%2F"))
    );
    let got = result.as_ref().unwrap();
    assert_eq!(
        got.evidence.children_count, 2,
        "empty string key remains a parsed ticket"
    );
    assert_eq!(got.evidence.children.omitted, 2);
    assert_eq!(reported(&effects(&result)), Outcome::Partial);
}

#[test]
fn children_missing_collections_and_unusable_continuation_are_partial() {
    for body in [
        json!({}),
        json!({"issues": null}),
        json!({"issues": [], "isLast": false}),
        json!({"issues": [issue("ABC-13")], "isLast": false}),
    ] {
        let (result, transport) = read(Routed::complete().body(CHILDREN, body));
        assert_eq!(transport.count(CHILDREN), 1);
        assert_eq!(
            result.as_ref().unwrap().evidence.children.incomplete_pages,
            1
        );
        assert_eq!(reported(&effects(&result)), Outcome::Partial);
    }
}

#[test]
fn missing_optional_issue_links_are_fine_but_present_malformed_collections_are_partial() {
    for malformed in [Value::Null, json!({}), json!("SECRET_BAD_COLLECTION")] {
        let mut main = issue("ABC-12");
        main["fields"]["issuelinks"] = malformed;
        let (result, _) = read(Routed::complete().body(ISSUE, main));
        assert!(
            result
                .as_ref()
                .unwrap()
                .evidence
                .issue_links
                .incomplete_collection
        );
        assert_eq!(reported(&effects(&result)), Outcome::Partial);
    }
    let (result, _) = read(Routed::complete());
    assert_eq!(reported(&effects(&result)), Outcome::Succeeded);
}

#[test]
fn issue_and_web_link_omissions_follow_the_exact_legacy_filters() {
    let mut main = issue("ABC-12");
    main["fields"]["issuelinks"] = json!([
        {}, {"outwardIssue": {"key": 3}}, {"outwardIssue": null, "inwardIssue": {"key": "ABC-1"}},
        {"inwardIssue": {"key": ""}}, {"outwardIssue": {"key": "SECRET-1"}},
    ]);
    let (result, _) = read(Routed::complete().body(ISSUE, main).body(WEB, json!([
        {}, {"object": {"url": 4}}, null, {"object": {"url": ""}}, {"object": {"url": "SECRET_NOT_A_URL"}},
    ])));
    let e = &result.as_ref().unwrap().evidence;
    assert_eq!((e.issue_link_count, e.issue_links.omitted_rows), (2, 3));
    assert_eq!((e.web_link_count, e.web.omitted_rows), (2, 3));
    assert_eq!(reported(&effects(&result)), Outcome::Partial);
    for effect in effects(&result) {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects.len(), 1);
            assert_eq!(action.subjects[0].key.id, "ABC-12");
            assert!(!format!("{action:?}").contains("SECRET"));
        }
    }
}

#[test]
fn web_wrapper_preserves_nonarrays_omissions_and_unvalidated_urls() {
    for body in [
        json!({}),
        Value::Null,
        json!([]),
        json!([{}, {"object": {"url": ""}}, {"object": {"url": "SECRET_URL"}}]),
    ] {
        let a = Arc::new(Routed::complete().body(WEB, body.clone()));
        let old = block(Client::over(a.clone()).web_links("ABC-12")).unwrap();
        let b = Arc::new(Routed::complete().body(WEB, body.clone()));
        let got = block(Client::over(b.clone()).web_links_observed("ABC-12")).unwrap();
        assert_eq!(old, got.links);
        assert_eq!(a.paths(), b.paths());
        assert_eq!(got.evidence.incomplete_collection, !body.is_array());
        assert_eq!(
            got.evidence.omitted_rows,
            usize::from(body.as_array().is_some_and(|rows| !rows.is_empty()))
        );
    }
}

#[test]
fn comments_keep_every_row_and_do_not_treat_older_or_oversized_results_as_cutoffs() {
    for (n, total) in [(50, 100), (51, 100), (51, 1)] {
        let (result, transport) = read(Routed::complete().body(
            COMMENTS,
            json!({"comments": vec![Value::Null; n], "total": total}),
        ));
        let got = result.as_ref().unwrap();
        assert_eq!(
            got.detail.comments.len(),
            n,
            "even malformed rows are retained"
        );
        assert_eq!(got.detail.comments_total, total);
        assert_eq!(transport.count(COMMENTS), 1);
        let fx = effects(&result);
        assert_eq!(
            reported(&fx),
            Outcome::Succeeded,
            "no semantic validation of comment fields or totals"
        );
        assert_eq!(
            observation(&fx, "jira.page.detail.comments"),
            Some((n as u32, Outcome::Succeeded, false, None))
        );
        assert!(
            fx.iter()
                .all(|e| !matches!(e, Effect::Observe { action, .. } if action.metadata.truncated))
        );
    }
}

#[test]
fn comments_wrapper_preserves_fallback_total_and_top_level_collection_defaults() {
    for body in [
        json!({}),
        json!({"comments": null, "total": 20}),
        json!({"comments": [comment(), null, {}]}),
        json!({"comments": [comment()], "total": "SECRET_TOTAL"}),
        json!({"comments": [comment()], "total": -1}),
    ] {
        let a = Arc::new(Routed::complete().body(COMMENTS, body.clone()));
        let expected = block(Client::over(a.clone()).comments("ABC-12", 7, 50)).unwrap();
        let b = Arc::new(Routed::complete().body(COMMENTS, body.clone()));
        let got = block(Client::over(b.clone()).comments_observed("ABC-12", 7, 50)).unwrap();
        assert_eq!(expected, (got.comments.clone(), got.total));
        assert_eq!(a.paths(), b.paths());
        assert_eq!(
            got.evidence.incomplete_collection,
            !body.get("comments").is_some_and(Value::is_array)
        );
        assert_eq!(
            got.evidence.total_defaulted,
            body.get("total").and_then(Value::as_u64).is_none()
        );
        if got.evidence.total_defaulted {
            assert_eq!(got.total, 7 + got.comments.len());
        }
        let (result, _) = read(Routed::complete().body(COMMENTS, body));
        assert_eq!(reported(&effects(&result)), Outcome::Partial);
    }
}

#[test]
fn development_skip_gaps_and_late_host_error_are_partial_page_results() {
    for id in ["", "SECRET_BAD_ID", "100/2"] {
        let mut main = issue("ABC-12");
        main["id"] = json!(id);
        let (result, transport) = read(Routed::complete().body(ISSUE, main));
        assert!(result.as_ref().unwrap().evidence.code.skipped_invalid_id);
        assert_eq!(transport.count(SUMMARY), 0);
        let fx = effects(&result);
        assert_eq!(reported(&fx), Outcome::Partial);
        assert_eq!(
            observation(&fx, "jira.page.detail.code.requests.skipped"),
            Some((1, Outcome::Blocked, false, Some(Reason::InvalidInput)))
        );
        assert!(observation(&fx, "jira.page.detail.code.links").is_none());
    }
    let (result, _) = read(
        Routed::complete()
            .body(SUMMARY, summary(json!({"A": {"count": 1}})))
            .body(
                CODE,
                json!({"detail": [{"pullRequests": [{}, {"url": ""}]}, {}]}),
            ),
    );
    let got = result.as_ref().unwrap();
    assert_eq!(got.detail.code.len(), 1);
    assert_eq!(got.evidence.code.omitted_rows, 1);
    assert_eq!(got.evidence.code.incomplete_responses, 1);
    assert_eq!(reported(&effects(&result)), Outcome::Partial);
    let (result, transport) = read(
        Routed::complete()
            .body(
                SUMMARY,
                summary(json!({"A": {"count": 1}, "B": {"count": 1}})),
            )
            .with(
                CODE,
                vec![
                    Reply::Response(
                        200,
                        json!({"detail": [{"pullRequests": [{"url": "SECRET_URL"}]}]}),
                    ),
                    Reply::Network,
                ],
            ),
    );
    assert_eq!(transport.count(CODE), 2);
    assert!(
        result.as_ref().unwrap().detail.code.is_empty(),
        "late code error still loses earlier links"
    );
    let fx = effects(&result);
    assert_eq!(reported(&fx), Outcome::Partial);
    assert!(observation(&fx, "jira.page.detail.code.links").is_none());
    assert_eq!(
        observation(&fx, "jira.page.detail.code.errors").unwrap().0,
        1
    );
}

#[test]
fn captures_requested_scope_and_omits_unsafe_identity_without_changing_behavior() {
    let requested = site();
    let scope = action(&requested, "ABC-12");
    assert_eq!(scope.subjects[0].key.instance, "https://jira.example/");
    assert_eq!(scope.subjects[0].key.id, "ABC-12");
    assert_eq!(scope.subjects[0].role, SubjectRole::Primary);
    assert!(scope.subjects[0].url.is_none());
    assert!(!format!("{scope:?}").contains("SECRET"));
    for (url, key) in [
        ("https://[", "ABC-12"),
        ("file:///SECRET", "ABC-12"),
        ("https://jira.example", "SECRET bad key"),
    ] {
        let mut other = requested.clone();
        other.url = url.into();
        let scope = action(&other, key);
        assert!(scope.subjects.is_empty());
        assert!(scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
    }
}

#[test]
fn no_op_fetch_and_manual_refresh_notice_remain_unchanged() {
    let mut plugin = demo::sample();
    assert!(page::fetch(&mut plugin, "missing", "ABC-12", false).is_empty());
    let fx = page::open(&mut plugin, "acme", "ABC-12");
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::SpawnAction { .. }))
            .count(),
        1
    );
    for force in [true, false] {
        assert!(page::fetch(&mut plugin, "acme", "ABC-12", force).is_empty());
    }
    let refresh = || Chord::new(Key::Char('r'));
    assert!(
        matches!(page::key(&mut plugin, refresh()), radar_core::plugin::Outcome::Handled(fx) if matches!(&fx[..], [Effect::Notify(text)] if text == "already asking Jira"))
    );
    let (got, _) = read(Routed::complete());
    crate::tests::complete_detail(&mut plugin, "acme", "ABC-12", Ok(got.unwrap().detail));
    assert!(page::fetch(&mut plugin, "acme", "ABC-12", false).is_empty());
    assert!(
        matches!(page::key(&mut plugin, refresh()), radar_core::plugin::Outcome::Handled(fx) if matches!(&fx[..], [Effect::SpawnAction { .. }]))
    );
    assert!(page::fetch(&mut plugin, "acme", "ABC-12", false).is_empty());
}

#[test]
fn client_setup_stays_inside_the_dispatched_task_and_fails_before_issue_guard() {
    let mut plugin = demo::sample();
    plugin.cfg.sites[0].email.clear();
    let mut fx = page::fetch(&mut plugin, "acme", "SECRET invalid key", false);
    assert!(plugin.details[&detail_key("acme", "SECRET invalid key")].loading);
    let Effect::SpawnAction { action, task } = fx.pop().unwrap() else {
        panic!("independent dispatch")
    };
    assert!(action.metadata.redacted);
    let msg = block(task);
    let JiraMsg::Detail {
        action: captured,
        result,
        ..
    } = *msg.downcast::<JiraMsg>().unwrap()
    else {
        panic!("detail reply")
    };
    assert_eq!(captured.subjects, action.subjects);
    assert!(result.as_ref().unwrap_err().contains("no email set"));
    assert_eq!(reported(&effects(&result)), Outcome::Failed);
}

#[test]
fn failed_development_read_preserves_cached_links_and_changed_ticket_retry() {
    let mut plugin = demo::sample();
    for data in plugin.data.values_mut() {
        for keys in data.found.values_mut() {
            keys.retain(|key| key == "ABC-12");
        }
    }
    let old = crate::CodeEntry {
        at: plugin.now - 10,
        updated: plugin.data["acme"].tickets["ABC-12"].updated.clone(),
        links: crate::tests::detail().code,
    };
    let mut mr = radar_core::Subject::new(crate::join::MERGE_REQUEST, "api!263", "Unrelated name");
    mr.url = Some(old.links[0].url.clone());
    plugin.data.get_mut("acme").unwrap().code.insert("ABC-12".into(), old.clone());
    assert!(crate::join::contribute(&plugin, radar_core::Point::Column, &mr).is_some());
    let (result, transport) = read(Routed::complete().with(SUMMARY, vec![Reply::Network]));
    assert!(result.as_ref().unwrap().evidence.code_failed);
    assert_eq!(transport.count(SUMMARY), 1);
    let owner = crate::tests::detail_read(&mut plugin, "acme", "ABC-12");
    let fx = plugin.on_msg(Box::new(JiraMsg::Detail {
        site: "acme".into(),
        key: "ABC-12".into(),
        action: action(plugin.cfg.site("acme").unwrap(), "ABC-12"),
        read: owner,
        result,
    }));
    assert_eq!(reported(&fx), Outcome::Partial);
    assert_eq!(plugin.data["acme"].tickets["ABC-12"].updated, "SECRET_UPDATED");
    assert_eq!(plugin.data["acme"].code["ABC-12"], old,
        "a failed optional development request cannot erase or refresh a successful cache");
    assert!(plugin.details[&detail_key("acme", "ABC-12")].value.as_ref().unwrap().code.is_empty(),
        "fresh visible detail keeps the client's optional empty fallback");
    assert!(crate::join::contribute(&plugin, radar_core::Point::Column, &mr).is_some(),
        "the unrelated-title relationship survives through cached development links");
    assert!(fx.iter().all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. })));
    assert_eq!(plugin.sync_code().len(), 1, "the changed ticket remains due under normal gates");
    assert!(plugin.sync.code_in_flight.contains(&detail_key("acme", "ABC-12")));
}

#[test]
fn optional_code_failure_preserves_cache_and_transient_evidence_is_not_persisted() {
    let mut observed_plugin = demo::sample();
    let mut legacy_plugin = demo::sample();
    for plugin in [&mut observed_plugin, &mut legacy_plugin] {
        page::open(plugin, "acme", "ABC-12");
    }
    let (result, _) = read(Routed::complete().with(SUMMARY, vec![Reply::Network]));
    let detail = result.as_ref().unwrap().detail.clone();
    let scope = action(observed_plugin.cfg.site("acme").unwrap(), "ABC-12");
    crate::tests::complete_detail(&mut legacy_plugin, "acme", "ABC-12", Ok(detail.clone()));
    // The page/ticket still installs identically, but the failed optional read
    // must not create a newly fresh code entry where none was held.
    legacy_plugin.data.get_mut("acme").unwrap().code.remove("ABC-12");
    let owner = crate::tests::detail_read(&mut observed_plugin, "acme", "ABC-12");
    let fx = observed_plugin.on_msg(Box::new(JiraMsg::Detail {
        site: "acme".into(),
        key: "ABC-12".into(),
        action: scope.clone(),
        read: owner,
        result,
    }));
    assert_eq!(reported(&fx), Outcome::Partial);
    assert!(
        fx.iter()
            .all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. })),
        "history adds no cache persistence/scheduling effect"
    );
    assert_eq!(observed_plugin.snapshot(), legacy_plugin.snapshot());
    let id = detail_key("acme", "ABC-12");
    assert_eq!(observed_plugin.details[&id].value, Some(detail));
    assert_eq!(
        observed_plugin.details[&id].at,
        legacy_plugin.details[&id].at
    );
    assert!(!observed_plugin.details[&id].loading);
    assert!(
        !observed_plugin.data["acme"].code.contains_key("ABC-12"),
        "optional code failure leaves an absent code cache absent"
    );
    let before = observed_plugin.snapshot();
    let at = observed_plugin.details[&id].at;
    observed_plugin.now += 9;
    let owner = crate::tests::detail_read(&mut observed_plugin, "acme", "ABC-12");
    let fx = observed_plugin.on_msg(Box::new(JiraMsg::Detail {
        site: "acme".into(),
        key: "ABC-12".into(),
        action: scope,
        read: owner,
        result: Err("SECRET_ERROR".into()),
    }));
    assert_eq!(reported(&fx), Outcome::Failed);
    assert_eq!(observed_plugin.snapshot(), before);
    assert_eq!(observed_plugin.details[&id].at, at);
    assert!(observed_plugin.details[&id].value.is_some());
    assert_eq!(
        observed_plugin.details[&id].error.as_deref(),
        Some("SECRET_ERROR")
    );
    let cache = before.unwrap().to_string();
    for field in [
        "DetailEvidence",
        "children_failed",
        "code_failed",
        "total_defaulted",
        "omitted_rows",
        "jira.page.detail",
        "SECRET_ERROR",
        "SECRET_NETWORK_ERROR",
    ] {
        assert!(!cache.contains(field));
    }
}

#[test]
fn numeric_counts_saturate_without_wrapping() {
    let effect = count(
        &action(&site(), "ABC-12"),
        "jira.page.detail.comments",
        usize::MAX,
        false,
        Outcome::Succeeded,
    );
    assert_eq!(
        observation(&[effect], "jira.page.detail.comments"),
        Some((u32::MAX, Outcome::Succeeded, usize::BITS > 32, None))
    );
}

mod host_tests;
mod open_tests;
