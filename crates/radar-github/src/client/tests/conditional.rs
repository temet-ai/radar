//! Conditional reads, the GraphQL "what moved" probe and rate-limit reports.

use super::*;
use crate::test_support::RepositoryBuilder;

fn tagged(mut response: Response, etag: &str) -> Response {
    response
        .headers
        .insert(ETAG, HeaderValue::from_str(etag).unwrap());
    response
}
fn not_modified() -> Response {
    Response {
        status: 304,
        headers: HeaderMap::new(),
        // Not JSON: a 304 that were parsed would fail the read.
        body: Ok(b"<not json>".to_vec()),
    }
}
fn etag(value: &str) -> Validator {
    Validator {
        etag: Some(value.into()),
        last_modified: None,
    }
}
const PULLS: &str = "https://api.github.com/repos/org/repo/pulls?state=open&sort=updated&direction=desc&per_page=100&page=";

#[tokio::test]
async fn the_etag_is_sent_back_and_a_304_keeps_the_list_with_nothing_parsed() {
    let (client, fixture) = client(vec![
        tagged(
            response(200, json!([pull(7, Some("org/repo"))])),
            "W/\"one\"",
        ),
        not_modified(),
    ]);
    let Fresh::New(pulls, tags) = client.open_pulls_if(&id("org/repo"), &[]).await.unwrap() else {
        panic!("a first read is new");
    };
    assert_eq!(pulls.items.len(), 1);
    assert_eq!(tags, [etag("W/\"one\"")]);
    assert_eq!(
        client.open_pulls_if(&id("org/repo"), &tags).await.unwrap(),
        Fresh::Same
    );
    assert_eq!(fixture.asked(), [None, Some("W/\"one\"".into())]);
    assert_eq!(fixture.paths(), [format!("{PULLS}1"), format!("{PULLS}1")]);
    let traffic = client.traffic();
    assert_eq!((traffic.requests, traffic.not_modified), (2, 1));
}

#[tokio::test]
async fn a_304_nobody_asked_for_is_refused() {
    let (client, _) = client(vec![not_modified()]);
    assert_eq!(
        client.open_pulls(&id("org/repo")).await.unwrap_err().kind,
        ApiErrorKind::InvalidResponse
    );
}

#[tokio::test]
async fn a_changed_list_is_read_with_its_new_validator() {
    let (client, fixture) = client(vec![tagged(
        response(200, json!([pull(8, Some("org/repo"))])),
        "\"two\"",
    )]);
    let fresh = client
        .open_pulls_if(&id("org/repo"), &[etag("\"one\"")])
        .await
        .unwrap();
    let Fresh::New(pulls, tags) = fresh else {
        panic!("a 200 is new");
    };
    assert_eq!(pulls.items[0].number, 8);
    assert_eq!(tags, [etag("\"two\"")]);
    assert_eq!(fixture.asked(), [Some("\"one\"".into())]);
}

#[tokio::test]
async fn a_later_page_that_moved_reads_the_whole_list_again() {
    let path = "/repos/org/repo/pulls";
    let (client, fixture) = client(vec![
        not_modified(),
        tagged(response(200, json!([pull(9, Some("org/repo"))])), "\"b2\""),
        tagged(page(json!([pull(7, Some("org/repo"))]), path, 2), "\"a2\""),
        tagged(response(200, json!([pull(9, Some("org/repo"))])), "\"b2\""),
    ]);
    let Fresh::New(pulls, tags) = client
        .open_pulls_if(&id("org/repo"), &[etag("\"a\""), etag("\"b\"")])
        .await
        .unwrap()
    else {
        panic!("a moved page is new");
    };
    assert_eq!(
        pulls.items.iter().map(|p| p.number).collect::<Vec<_>>(),
        [7, 9]
    );
    assert_eq!(tags, [etag("\"a2\""), etag("\"b2\"")]);
    assert_eq!(
        fixture.asked(),
        [Some("\"a\"".into()), Some("\"b\"".into()), None, None]
    );
}

#[tokio::test]
async fn an_incomplete_or_untagged_list_keeps_no_validators() {
    let (plain, _) = client(vec![response(200, json!([pull(7, Some("org/repo"))]))]);
    let Fresh::New(_, tags) = plain.open_pulls_if(&id("org/repo"), &[]).await.unwrap() else {
        panic!("new");
    };
    assert!(tags.is_empty(), "no ETag, nothing to ask with");
    let path = "/repos/org/repo/pulls";
    let (partial, _) = client(vec![
        tagged(page(json!([pull(7, Some("org/repo"))]), path, 2), "\"a\""),
        Response {
            status: 502,
            headers: HeaderMap::new(),
            body: Ok(vec![]),
        },
    ]);
    let Fresh::New(pulls, tags) = partial.open_pulls_if(&id("org/repo"), &[]).await.unwrap() else {
        panic!("new");
    };
    assert!(!pulls.completion.complete);
    assert!(tags.is_empty(), "a partial read is never the reference");
}

fn runs(status: &str, conclusion: Option<&str>) -> Value {
    json!({"total_count": 1, "check_runs": [check(1, SHA, status, conclusion)]})
}

#[tokio::test]
async fn a_304_pull_proves_the_held_head_and_unchanged_checks_are_same() {
    let (client, fixture) = client(vec![
        tagged(response(200, pull(7, Some("org/repo"))), "\"p\""),
        tagged(response(200, runs("completed", Some("success"))), "\"c\""),
        tagged(
            response(200, json!([status(2, "ci", "success", "org/repo")])),
            "\"s\"",
        ),
        not_modified(),
        not_modified(),
        not_modified(),
    ]);
    let Fresh::New(held, Some(tag)) = client.pull_if(&id("org/repo"), 7, None).await.unwrap()
    else {
        panic!("new pull with its validator");
    };
    let Fresh::New(checks, tags) = client.checks_if(&held, None).await.unwrap() else {
        panic!("new checks");
    };
    assert_eq!(tags.len(), 2);
    assert_eq!(
        client
            .pull_if(&id("org/repo"), 7, Some((&held, &tag)))
            .await
            .unwrap(),
        Fresh::Same
    );
    assert_eq!(
        client
            .checks_if(&held, Some((&checks, &tags)))
            .await
            .unwrap(),
        Fresh::Same
    );
    assert_eq!(
        fixture.asked()[3..],
        [
            Some("\"p\"".into()),
            Some("\"c\"".into()),
            Some("\"s\"".into())
        ]
    );
}

#[tokio::test]
async fn one_check_list_changed_keeps_the_rows_held_for_the_other() {
    let (client, _) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        tagged(response(200, runs("in_progress", None)), "\"c\""),
        tagged(
            response(200, json!([status(2, "ci", "success", "org/repo")])),
            "\"s\"",
        ),
        tagged(response(200, runs("completed", Some("success"))), "\"c2\""),
        not_modified(),
    ]);
    let held = client.pull(&id("org/repo"), 7).await.unwrap();
    let Fresh::New(checks, tags) = client.checks_if(&held, None).await.unwrap() else {
        panic!("new");
    };
    assert!(checks.running());
    let Fresh::New(now, now_tags) = client
        .checks_if(&held, Some((&checks, &tags)))
        .await
        .unwrap()
    else {
        panic!("the runs changed");
    };
    assert!(!now.running());
    assert_eq!(now.statuses.items, checks.statuses.items);
    assert!(now.statuses.completion.complete);
    assert_eq!(now_tags["org/repo/check-runs"], [etag("\"c2\"")]);
    assert_eq!(now_tags["org/repo/statuses"], [etag("\"s\"")]);
}

#[tokio::test]
async fn markers_ask_many_repositories_in_one_read_only_graphql_query() {
    let fixture = FixtureTransport::with_auth(
        vec![response(
            200,
            json!({"data": {
                "r0": {"nameWithOwner": "Org/Repo", "pullRequests": {"totalCount": 2, "nodes": [{"updatedAt": "2026-10-11T12:00:00Z"}]}},
                "r1": null
            }}),
        )],
        true,
    );
    let client = Client::with_transport(
        &[id("org/repo"), id("org/gone")],
        "github_pat_synthetic_token_123",
        fixture.clone(),
    )
    .unwrap();
    let markers = client
        .markers(&[id("org/repo"), id("org/gone")])
        .await
        .unwrap();
    assert_eq!(
        markers,
        [
            (
                id("org/repo"),
                Some(Marker {
                    open: 2,
                    latest: Some("2026-10-11T12:00:00Z".into())
                })
            ),
            (id("org/gone"), None),
        ]
    );
    let bodies = fixture.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "one request for every repository");
    let query = bodies[0]["query"].as_str().unwrap();
    assert!(query.starts_with("query("));
    assert!(!query.contains("mutation"));
    assert!(!query.contains("org"), "names travel as variables only");
    assert_eq!(bodies[0]["variables"]["o1"], "org");
    assert_eq!(bodies[0]["variables"]["n1"], "gone");
    assert_eq!(
        client.markers(&[id("elsewhere/x")]).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    let (public, _) = client_of(vec![]);
    assert_eq!(
        public.markers(&[id("org/repo")]).await.unwrap_err().kind,
        ApiErrorKind::MissingToken
    );
}
fn client_of(responses: Vec<Response>) -> (Client, Arc<FixtureTransport>) {
    client(responses)
}

#[tokio::test]
async fn a_graphql_rate_limit_in_a_200_is_a_rate_limit() {
    let mut limited = response(
        200,
        json!({"data": null, "errors": [{"type": "RATE_LIMITED", "message": "API rate limit exceeded"}]}),
    );
    limited
        .headers
        .insert("x-ratelimit-reset", HeaderValue::from_static("1791723600"));
    let fixture = FixtureTransport::with_auth(vec![limited], true);
    let client =
        Client::with_transport(&[id("org/repo")], "github_pat_synthetic_token_123", fixture)
            .unwrap();
    let err = client.markers(&[id("org/repo")]).await.unwrap_err();
    assert_eq!(err.kind, ApiErrorKind::RateLimit);
    assert_eq!(err.rate_limit_reset, Some(1791723600));
}

#[tokio::test]
async fn every_response_reports_its_rate_limit_bucket() {
    let mut first = not_modified();
    for (name, value) in [
        ("x-ratelimit-limit", "5000"),
        ("x-ratelimit-remaining", "4990"),
        ("x-ratelimit-reset", "1791723600"),
        ("x-ratelimit-resource", "core"),
    ] {
        first.headers.insert(name, HeaderValue::from_static(value));
    }
    let (client, _) = client(vec![first]);
    assert_eq!(
        client
            .open_pulls_if(&id("org/repo"), &[etag("\"a\"")])
            .await
            .unwrap(),
        Fresh::Same
    );
    assert_eq!(
        client.traffic().rates,
        [RateLimit {
            resource: "core".into(),
            limit: 5000,
            remaining: 4990,
            reset: 1791723600,
        }]
    );
}

#[tokio::test]
async fn a_body_goes_only_to_the_graphql_endpoint() {
    let transport = NativeTransport::new(&Endpoint::default()).unwrap();
    let result = transport
        .send(Request {
            url: "https://api.github.com/repos/org/repo".into(),
            headers: HeaderMap::new(),
            authorization: None,
            graphql: Some(b"{}".to_vec()),
        })
        .await;
    assert_eq!(result.err().unwrap().kind, ApiErrorKind::InvalidTarget);
}

/// A GraphQL answer for a batch of `count` repositories, each with one
/// marker; the transport yields while a request is open, so overlapping
/// requests are seen as such.
struct Overlap {
    open: Mutex<usize>,
    most: Mutex<usize>,
    batches: Mutex<Vec<usize>>,
    refuse_after: Option<usize>,
}
impl Transport for Overlap {
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            let body: Value = serde_json::from_slice(&request.graphql.unwrap()).unwrap();
            let count = body["variables"].as_object().unwrap().len() / 2;
            let nth = {
                let mut batches = self.batches.lock().unwrap();
                batches.push(count);
                batches.len()
            };
            {
                let mut open = self.open.lock().unwrap();
                *open += 1;
                let mut most = self.most.lock().unwrap();
                *most = (*most).max(*open);
            }
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            *self.open.lock().unwrap() -= 1;
            if self.refuse_after.is_some_and(|n| nth > n) {
                let mut limited = response(
                    200,
                    json!({"data": null, "errors": [{"type": "RATE_LIMITED"}]}),
                );
                limited
                    .headers
                    .insert("x-ratelimit-reset", HeaderValue::from_static("1791723600"));
                return Ok(limited);
            }
            let data: serde_json::Map<String, Value> = (0..count)
                .map(|i| {
                    let name = body["variables"][format!("n{i}")].as_str().unwrap();
                    let owner = body["variables"][format!("o{i}")].as_str().unwrap();
                    (
                        format!("r{i}"),
                        json!({"nameWithOwner": format!("{owner}/{name}"),
                            "pullRequests": {"totalCount": 0, "nodes": []}}),
                    )
                })
                .collect();
            Ok(response(200, json!({ "data": data })))
        })
    }
}

fn many(count: usize) -> Vec<RepositoryId> {
    (0..count).map(|i| id(&format!("org/repo{i}"))).collect()
}
fn overlap(refuse_after: Option<usize>) -> Arc<Overlap> {
    Arc::new(Overlap {
        open: Mutex::new(0),
        most: Mutex::new(0),
        batches: Mutex::new(vec![]),
        refuse_after,
    })
}

#[tokio::test]
async fn the_probe_batches_of_a_round_are_in_flight_together() {
    let transport = overlap(None);
    let ids = many(76);
    let client =
        Client::with_transport(&ids, "github_pat_synthetic_token_123", transport.clone()).unwrap();
    let markers = client.markers(&ids).await.unwrap();
    assert_eq!(
        markers.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
        ids,
        "every repository answered, in the order asked"
    );
    let mut batches = transport.batches.lock().unwrap().clone();
    batches.sort();
    assert_eq!(
        batches,
        [26, 50],
        "76 repositories are batches of 50 and 26"
    );
    assert_eq!(
        *transport.most.lock().unwrap(),
        2,
        "the second batch does not wait for the first"
    );
}

#[tokio::test]
async fn no_more_batches_than_the_cap_are_in_flight() {
    let transport = overlap(None);
    let ids = many(150);
    let client =
        Client::with_transport(&ids, "github_pat_synthetic_token_123", transport.clone()).unwrap();
    assert_eq!(client.markers(&ids).await.unwrap().len(), 150);
    assert_eq!(transport.batches.lock().unwrap().len(), 3);
    // Nothing held for any of 200: full reads, in batches of 25, eight of them.
    let ids = many(MAX_REPOSITORIES);
    let transport = overlap(None);
    let client =
        Client::with_transport(&ids, "github_pat_synthetic_token_123", transport.clone()).unwrap();
    let asks: Vec<_> = ids.iter().map(|id| (id.clone(), true)).collect();
    assert_eq!(client.probe(&asks).await.unwrap().len(), 200);
    assert_eq!(transport.batches.lock().unwrap().clone(), [25; 8]);
    assert_eq!(
        *transport.most.lock().unwrap(),
        4,
        "eight batches, four at a time"
    );
}

#[tokio::test]
async fn a_refused_batch_ends_the_probe_with_its_rate_limit_guidance() {
    let transport = overlap(Some(1));
    let ids = many(120);
    let client = Client::with_transport(&ids, "github_pat_synthetic_token_123", transport).unwrap();
    let err = client.markers(&ids).await.unwrap_err();
    assert_eq!(err.kind, ApiErrorKind::RateLimit);
    assert_eq!(
        err.rate_limit_reset,
        Some(1791723600),
        "the wait is still named"
    );
}

fn node(number: u64) -> Value {
    json!({"number": number, "title": "Sample PR", "body": "Description", "isDraft": false,
        "headRefOid": SHA.to_uppercase(), "headRefName": "topic", "baseRefName": "main",
        "updatedAt": "2026-10-11T12:00:00Z", "author": {"login": "author"},
        "headRepository": {"nameWithOwner": "Org/Repo"}})
}
fn full_repo(nodes: Vec<Value>, total: u64) -> Value {
    json!({"nameWithOwner": "org/repo", "description": "sample", "isPrivate": false,
        "isArchived": false, "defaultBranchRef": {"name": "main"},
        "pullRequests": {"totalCount": total, "nodes": nodes}})
}

#[tokio::test]
async fn a_repository_asked_in_full_comes_back_as_its_rest_reads_would() {
    // The full read goes first, in a batch of its own; the marker follows.
    let fixture = FixtureTransport::with_auth(
        vec![
            response(200, json!({"data": {"r0": full_repo(vec![node(7)], 1)}})),
            response(
                200,
                json!({"data": {"r0": {"nameWithOwner": "org/warm",
                    "pullRequests": {"totalCount": 3, "nodes": [{"updatedAt": "2026-10-11T12:00:00Z"}]}}}}),
            ),
        ],
        true,
    );
    let ids = [id("org/repo"), id("org/warm")];
    let client =
        Client::with_transport(&ids, "github_pat_synthetic_token_123", fixture.clone()).unwrap();
    let probed = client
        .probe(&[(ids[0].clone(), true), (ids[1].clone(), false)])
        .await
        .unwrap();
    assert_eq!(
        fixture.paths().len(),
        2,
        "full reads and markers are batched apart"
    );
    assert_eq!(probed[0].id, ids[0], "and come back in the order asked");
    let full = probed[0].full.clone().expect("asked in full");
    assert_eq!(probed[0].marker.as_ref().unwrap().open, 1);
    let from_rest = serde_json::from_value::<RawPull>(pull(7, Some("org/repo")))
        .unwrap()
        .checked(&id("org/repo"), None)
        .unwrap();
    assert_eq!(
        full.pulls.items,
        [from_rest],
        "the same PullRequest either way"
    );
    assert!(full.pulls.completion.complete);
    assert_eq!(
        full.repository,
        RepositoryBuilder::default()
            .id(id("org/repo"))
            .description("sample")
            .build()
    );
    assert!(
        probed[1].full.is_none(),
        "a repository not asked in full has only its marker"
    );
    assert!(probed[1].marker.is_some());
    let bodies = fixture.bodies.lock().unwrap().clone();
    let queries: Vec<_> = bodies
        .iter()
        .map(|b| b["query"].as_str().unwrap())
        .collect();
    assert!(
        queries[0].contains("headRefOid"),
        "the full fields are asked for the full read"
    );
    assert!(
        !queries[1].contains("headRefOid"),
        "and for no other: {}",
        queries[1]
    );
    assert!(queries.iter().all(|q| !q.contains("mutation")));
}

#[tokio::test]
async fn a_full_read_that_cannot_be_exact_leaves_the_repository_to_rest() {
    let mut ghost = node(8);
    ghost["author"] = Value::Null;
    ghost["headRepository"] = Value::Null;
    ghost["body"] = json!("");
    let mut bad_sha = node(9);
    bad_sha["headRefOid"] = json!("not-a-sha");
    let mut headless = full_repo(vec![], 0);
    headless["defaultBranchRef"] = Value::Null;
    let fixture = FixtureTransport::with_auth(
        vec![response(
            200,
            json!({"data": {
                "r0": full_repo(vec![ghost], 1),
                "r1": full_repo(vec![node(1)], 31),
                "r2": full_repo(vec![bad_sha], 1),
                "r3": headless,
                "r4": full_repo(vec![node(1), node(1)], 2),
            }}),
        )],
        true,
    );
    let me = id("org/repo");
    let client = Client::with_transport(
        std::slice::from_ref(&me),
        "github_pat_synthetic_token_123",
        fixture,
    )
    .unwrap();
    // Five asks of the one configured repository: the answer is by position.
    let probed = client.probe(&vec![(me.clone(), true); 5]).await.unwrap();
    let pull = &probed[0].full.as_ref().unwrap().pulls.items[0];
    assert_eq!(
        pull.author, "unknown",
        "a deleted account is `unknown`, as REST has it"
    );
    assert_eq!(
        pull.head_repository, None,
        "a deleted fork has no head repository"
    );
    assert_eq!(
        pull.body, None,
        "an empty description is none, as REST has it"
    );
    assert_eq!(pull.state, "open");
    for (i, why) in [
        (1, "31 open with 30 asked: REST pages them"),
        (2, "a head that is not a sha"),
        (3, "no default branch"),
        (4, "the same pull request twice"),
    ] {
        assert!(probed[i].full.is_none(), "{why}");
    }
}
