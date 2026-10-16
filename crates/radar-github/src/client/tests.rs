use super::*;
use serde_json::{Value, json};
use std::collections::VecDeque;

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

mod conditional;
mod endpoint;

#[derive(Default)]
struct FixtureTransport {
    responses: Mutex<VecDeque<Result<Response, ApiError>>>,
    requests: Mutex<Vec<String>>,
    /// The `If-None-Match` of each request, in order.
    asked: Mutex<Vec<Option<String>>>,
    bodies: Mutex<Vec<Value>>,
    authorized: bool,
}
impl FixtureTransport {
    fn new(responses: Vec<Response>) -> Arc<Self> {
        Self::with_auth(responses, false)
    }
    fn with_auth(responses: Vec<Response>, authorized: bool) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            authorized,
            ..Self::default()
        })
    }
    fn paths(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
    fn asked(&self) -> Vec<Option<String>> {
        self.asked.lock().unwrap().clone()
    }
}
impl Transport for FixtureTransport {
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            self.asked.lock().unwrap().push(
                request
                    .headers
                    .get(IF_NONE_MATCH)
                    .map(|v| v.to_str().unwrap().to_string()),
            );
            if let Some(body) = &request.graphql {
                assert_eq!(request.url, "https://api.github.com/graphql");
                assert_eq!(request.headers[CONTENT_TYPE], "application/json");
                self.bodies
                    .lock()
                    .unwrap()
                    .push(serde_json::from_slice(body).unwrap());
            }
            assert!(request.url.starts_with("https://api.github.com/"));
            assert_eq!(request.headers["accept"], "application/vnd.github+json");
            assert_eq!(request.headers["x-github-api-version"], "2022-11-28");
            assert_eq!(request.headers["user-agent"], "radar-github-read-only/0.1");
            assert_eq!(request.authorization.is_some(), self.authorized);
            assert!(
                request
                    .authorization
                    .as_ref()
                    .is_none_or(HeaderValue::is_sensitive)
            );
            self.requests.lock().unwrap().push(request.url);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra HTTP request")
        })
    }
}

#[tokio::test]
async fn token_check_and_accessible_repo_picker_need_no_configured_repository() {
    let mut first = repository("Alice/One");
    first["id"] = json!(1);
    let mut second = repository("acme-org/Two");
    second["id"] = json!(2);
    let fixture = FixtureTransport::with_auth(
        vec![
            response(200, json!({"id": 42, "login": "alice"})),
            response(200, json!([first, second])),
        ],
        true,
    );
    let client =
        Client::with_transport(&[], "github_pat_synthetic_token_123", fixture.clone()).unwrap();
    assert_eq!(
        client.check_connection().await.unwrap(),
        Some("alice".into())
    );
    let found = client.accessible_repositories().await.unwrap();
    assert!(found.completion.complete);
    assert_eq!(found.items, [id("alice/one"), id("acme-org/two")]);
    assert_eq!(
        fixture.paths(),
        [
            "https://api.github.com/user",
            "https://api.github.com/user/repos?sort=full_name&direction=asc&per_page=100&page=1",
        ]
    );
}

/// A classic or gh token's scopes come with the check's own answer; a
/// fine-grained token's do not, and none are made up.
#[tokio::test]
async fn the_token_check_reads_oauth_scopes_only_when_github_sends_them() {
    let mut classic = response(200, json!({"id": 42, "login": "vimes"}));
    classic.headers.insert("x-oauth-scopes", HeaderValue::from_static("read:org, public_repo"));
    let mut none = response(200, json!({"id": 42, "login": "vimes"}));
    none.headers.insert("x-oauth-scopes", HeaderValue::from_static(""));
    let fine_grained = response(200, json!({"id": 42, "login": "vimes"}));
    let fixture = FixtureTransport::with_auth(vec![classic, none, fine_grained], true);
    let client = Client::with_transport(&[], "github_pat_synthetic_token_123", fixture.clone()).unwrap();
    assert_eq!(
        client.check_connection_scopes().await.unwrap(),
        (Some("vimes".into()), Some(vec!["read:org".into(), "public_repo".into()]))
    );
    assert_eq!(client.check_connection_scopes().await.unwrap(), (Some("vimes".into()), Some(vec![])));
    assert_eq!(client.check_connection_scopes().await.unwrap(), (Some("vimes".into()), None));
    assert_eq!(fixture.paths(), ["https://api.github.com/user"; 3], "one request each, as before");
}

#[tokio::test]
async fn public_connection_check_works_without_repositories() {
    let (client, fixture) = client(vec![response(
        200,
        json!({"resources": {"core": {"remaining": 50}}}),
    )]);
    assert_eq!(client.check_connection().await.unwrap(), None);
    assert_eq!(fixture.paths(), ["https://api.github.com/rate_limit"]);
}
fn id(name: &str) -> RepositoryId {
    RepositoryId::parse(name).unwrap()
}
fn client(responses: Vec<Response>) -> (Client, Arc<FixtureTransport>) {
    let fixture = FixtureTransport::new(responses);
    (
        Client::with_transport(&[id("org/repo")], "", fixture.clone()).unwrap(),
        fixture,
    )
}
fn response(status: u16, body: Value) -> Response {
    Response {
        status,
        headers: HeaderMap::new(),
        body: Ok(serde_json::to_vec(&body).unwrap()),
    }
}
fn page(body: Value, path: &str, next: u32) -> Response {
    let mut response = response(200, body);
    response.headers.insert(
        LINK,
        HeaderValue::from_str(&format!(
            "<https://api.github.com{path}?per_page=100&page={next}>; rel=\"next\""
        ))
        .unwrap(),
    );
    response
}
fn repository(name: &str) -> Value {
    let (owner, repo) = name.split_once('/').unwrap();
    json!({"full_name": name, "name": repo, "owner": {"login": owner}, "html_url": format!("https://github.com/{name}"), "description": "sample", "default_branch": "main", "private": false, "archived": false})
}
fn pull(number: u64, head_repo: Option<&str>) -> Value {
    json!({"number": number, "html_url": format!("https://github.com/org/repo/pull/{number}"), "title": "Sample PR", "body": "Description", "user": {"login": "author"}, "state": "open", "draft": false, "head": {"sha": SHA, "ref": "topic", "repo": head_repo.map(repository)}, "base": {"sha": "b".repeat(40), "ref": "main", "repo": repository("org/repo")}, "updated_at": "2026-10-11T12:00:00Z", "merged": false})
}
fn check(id: u64, sha: &str, status: &str, conclusion: Option<&str>) -> Value {
    json!({"id": id, "name": "build", "head_sha": sha, "status": status, "conclusion": conclusion})
}
fn status(id: u64, context: &str, state: &str, repo: &str) -> Value {
    json!({"id": id, "url": format!("https://api.github.com/repos/{repo}/statuses/{SHA}"), "context": context, "state": state, "description": "build status"})
}

#[tokio::test]
async fn repository_and_open_pulls_use_only_configured_get_paths() {
    let (client, fixture) = client(vec![
        response(200, repository("Org/Repo")),
        response(200, json!([pull(7, Some("org/repo"))])),
    ]);
    let repo = client.repository(&id("ORG/REPO")).await.unwrap();
    assert_eq!(repo.id, id("org/repo"));
    let pulls = client.open_pulls(&repo.id).await.unwrap();
    assert_eq!(pulls.items.len(), 1);
    assert!(pulls.completion.complete);
    assert_eq!(
        fixture.paths(),
        [
            "https://api.github.com/repos/org/repo",
            "https://api.github.com/repos/org/repo/pulls?state=open&sort=updated&direction=desc&per_page=100&page=1"
        ]
    );
}

#[tokio::test]
async fn arbitrary_unconfigured_or_mutated_id_never_reaches_transport() {
    let (client, fixture) = client(vec![]);
    for repo in [
        id("elsewhere/repo"),
        RepositoryId {
            owner: "org".into(),
            name: "repo/../../secrets".into(),
        },
    ] {
        assert_eq!(
            client.repository(&repo).await.unwrap_err().kind,
            ApiErrorKind::InvalidTarget
        );
        assert_eq!(
            client.open_pulls(&repo).await.unwrap_err().kind,
            ApiErrorKind::InvalidTarget
        );
        assert_eq!(
            client.pull(&repo, 1).await.unwrap_err().kind,
            ApiErrorKind::InvalidTarget
        );
    }
    assert_eq!(
        client.pull(&id("org/repo"), 0).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    assert!(fixture.paths().is_empty());
}

#[tokio::test]
async fn metadata_mismatched_identity_and_unsafe_urls_are_rejected() {
    for value in [
        repository("other/repo"),
        {
            let mut repo = repository("org/repo");
            repo["html_url"] = json!("https://github.com@evil.example/org/repo");
            repo
        },
        {
            let mut repo = repository("org/repo");
            repo["owner"]["login"] = json!("other");
            repo
        },
    ] {
        let (client, _) = client(vec![response(200, value)]);
        assert_eq!(
            client.repository(&id("org/repo")).await.unwrap_err().kind,
            ApiErrorKind::InvalidResponse
        );
    }
}

#[tokio::test]
async fn detail_rejects_number_base_url_sha_and_missing_merge_evidence() {
    let mut cases = vec![];
    for (field, value) in [
        ("number", json!(8)),
        ("html_url", json!("https://github.com/org/repo/pull/8")),
        (
            "html_url",
            json!("https://github.com/org/repo/pull/7?token=secret"),
        ),
    ] {
        let mut raw = pull(7, Some("org/repo"));
        raw[field] = value;
        cases.push(raw);
    }
    let mut raw = pull(7, Some("org/repo"));
    raw["base"]["repo"] = repository("other/repo");
    cases.push(raw);
    let mut raw = pull(7, Some("org/repo"));
    raw["head"]["sha"] = json!("topic");
    cases.push(raw);
    let mut raw = pull(7, Some("org/repo"));
    raw.as_object_mut().unwrap().remove("merged");
    cases.push(raw);
    for raw in cases {
        let (client, fixture) = client(vec![response(200, raw)]);
        assert_eq!(
            client.pull(&id("org/repo"), 7).await.unwrap_err().kind,
            ApiErrorKind::InvalidResponse
        );
        assert_eq!(fixture.paths().len(), 1);
    }
}

#[tokio::test]
async fn fork_checks_preserve_base_and_source_provenance_at_exact_head_sha() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("contributor/fork"))),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("failure"))]}),
        ),
        response(200, json!([status(20, "CI", "failure", "org/repo")])),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(11, SHA, "completed", Some("success"))]}),
        ),
        response(
            200,
            json!([
                status(22, "ci", "pending", "contributor/fork"),
                status(21, "CI", "success", "contributor/fork")
            ]),
        ),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(checks.head_repository, Some(id("contributor/fork")));
    assert_eq!(checks.head_sha, SHA);
    assert!(checks.check_runs.completion.complete);
    assert!(checks.statuses.completion.complete);
    assert_eq!(checks.check_runs.completion.pages, 2);
    assert_eq!(checks.statuses.completion.pages, 2);
    assert_eq!(checks.check_runs.items.len(), 2);
    assert_eq!(checks.statuses.items.len(), 2);
    assert_eq!(checks.check_runs.items[0].repository, id("org/repo"));
    assert_eq!(
        checks.check_runs.items[1].repository,
        id("contributor/fork")
    );
    assert_eq!(checks.statuses.items[0].repository, id("org/repo"));
    assert_eq!(checks.statuses.items[0].state, "failure");
    assert_eq!(checks.statuses.items[1].repository, id("contributor/fork"));
    assert_eq!(checks.statuses.items[1].state, "pending");
    let paths = fixture.paths();
    assert_eq!(paths.len(), 5);
    for (index, repository, endpoint) in [
        (1, "org/repo", "check-runs"),
        (2, "org/repo", "statuses"),
        (3, "contributor/fork", "check-runs"),
        (4, "contributor/fork", "statuses"),
    ] {
        assert!(paths[index].starts_with(&format!(
            "https://api.github.com/repos/{repository}/commits/{SHA}/{endpoint}?"
        )));
    }
    assert!(
        paths
            .iter()
            .all(|path| !path.contains("merge") && !path.contains("topic"))
    );
}

#[tokio::test]
async fn checks_require_live_detail_proof_and_reject_mutated_head_or_number() {
    let (client, fixture) = client(vec![response(200, pull(7, Some("contributor/fork")))]);
    let raw: RawPull = serde_json::from_value(pull(7, Some("contributor/fork"))).unwrap();
    let unproven = raw.checked(&id("org/repo"), Some(7)).unwrap();
    assert_eq!(
        client.checks(&unproven).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    let original = client.pull(&id("org/repo"), 7).await.unwrap();
    let mut modified = original.clone();
    modified.head_repository = Some(id("another/repo"));
    assert_eq!(
        client.checks(&modified).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    modified = original.clone();
    modified.head_sha = "b".repeat(40);
    assert_eq!(
        client.checks(&modified).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    modified = original;
    modified.number = 8;
    assert_eq!(
        client.checks(&modified).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    assert_eq!(fixture.paths().len(), 1);
}

#[tokio::test]
async fn failed_detail_refresh_invalidates_older_proof() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(401, json!({"message":"secret must not escape"})),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    assert!(client.pull(&id("org/repo"), 7).await.is_err());
    assert_eq!(
        client.checks(&pull).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    assert_eq!(fixture.paths().len(), 2);
}

#[tokio::test]
async fn deleted_fork_retains_base_checks_and_marks_source_unknown() {
    let (client, fixture) = client(vec![
        response(200, pull(7, None)),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("success"))]}),
        ),
        response(200, json!([status(20, "ci", "success", "org/repo")])),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert!(!checks.check_runs.completion.complete);
    assert!(!checks.statuses.completion.complete);
    assert_eq!(checks.check_runs.items.len(), 1);
    assert_eq!(checks.statuses.items.len(), 1);
    assert_eq!(checks.check_runs.items[0].repository, id("org/repo"));
    assert_eq!(checks.statuses.items[0].repository, id("org/repo"));
    assert!(
        checks
            .errors
            .iter()
            .all(|error| error.contains("source repository"))
    );
    assert_eq!(fixture.paths().len(), 3);
}

#[tokio::test]
async fn wrong_check_sha_and_status_scope_are_not_accepted() {
    let (client, _) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, &"b".repeat(40), "completed", Some("success"))]}),
        ),
        response(200, json!([status(20, "ci", "success", "other/repo")])),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert!(!checks.check_runs.completion.complete);
    assert!(!checks.statuses.completion.complete);
    assert!(checks.check_runs.items.is_empty());
    assert!(checks.statuses.items.is_empty());
    assert_eq!(checks.errors.len(), 2);
}

#[tokio::test]
async fn checks_permission_failure_does_not_hide_independent_legacy_statuses() {
    let (client, _) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(403, json!({"message":"Resource not accessible"})),
        response(200, json!([status(1, "ci", "success", "org/repo")])),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert!(!checks.check_runs.completion.complete);
    assert!(checks.statuses.completion.complete);
    assert_eq!(checks.statuses.items.len(), 1);
}

#[tokio::test]
async fn rate_limit_stops_additional_head_requests_and_is_not_empty_success() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("contributor/fork"))),
        response(429, json!({"message":"rate limited"})),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert!(!checks.check_runs.completion.complete);
    assert!(!checks.statuses.completion.complete);
    assert_eq!(fixture.paths().len(), 2);
}

#[tokio::test]
async fn pagination_follows_evidence_with_own_safe_paths() {
    let (client, fixture) = client(vec![
        page(
            json!([pull(7, Some("org/repo"))]),
            "/repositories/42/pulls",
            2,
        ),
        response(200, json!([pull(8, Some("org/repo"))])),
    ]);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), 2);
    assert_eq!(pulls.completion.pages, 2);
    assert!(pulls.completion.complete);
    assert!(fixture.paths()[1].contains("/repos/org/repo/pulls?"));
    assert!(fixture.paths()[1].ends_with("page=2"));
}

#[tokio::test]
async fn page_cap_preserves_rows_and_explicit_partialness() {
    let responses = (1..=MAX_PAGES)
        .map(|n| {
            page(
                json!([pull(u64::from(n), Some("org/repo"))]),
                "/repos/org/repo/pulls",
                n + 1,
            )
        })
        .collect();
    let (client, fixture) = client(responses);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), MAX_PAGES as usize);
    assert_eq!(pulls.completion.pages, MAX_PAGES);
    assert_eq!(pulls.completion.reason.as_deref(), Some(PAGE_CAP));
    assert!(!pulls.completion.complete);
    assert_eq!(fixture.paths().len(), MAX_PAGES as usize);
}

#[tokio::test]
async fn failed_later_page_preserves_earlier_results_as_partial() {
    let (client, _) = client(vec![
        page(
            json!([pull(7, Some("org/repo"))]),
            "/repos/org/repo/pulls",
            2,
        ),
        response(401, json!({"message":"a synthetic private credential"})),
    ]);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), 1);
    assert!(!pulls.completion.complete);
    assert_eq!(pulls.completion.pages, 1);
    assert!(!pulls.completion.reason.unwrap().contains("synthetic"));
}

#[tokio::test]
async fn duplicate_pagination_rows_mark_snapshot_partial() {
    let (client, _) = client(vec![
        page(
            json!([pull(7, Some("org/repo"))]),
            "/repos/org/repo/pulls",
            2,
        ),
        response(200, json!([pull(7, Some("org/repo"))])),
    ]);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), 1);
    assert!(!pulls.completion.complete);
}

#[tokio::test]
async fn hostile_links_are_never_followed_and_never_claim_complete() {
    for target in [
        "https://evil.example/repos/org/repo/pulls?page=2",
        "https://api.github.com/repos/other/repo/pulls?page=2",
        "https://api.github.com/repos/org/repo/pulls?page=1",
        "https://api.github.com@evil.example/repos/org/repo/pulls?page=2",
        "https://api.github.com/repos/org/repo/pulls?page=2&page=3",
        "https://api.github.com/repos/org/repo/pulls?page=2#x",
    ] {
        let mut response = response(200, json!([pull(7, Some("org/repo"))]));
        response.headers.insert(
            LINK,
            HeaderValue::from_str(&format!("<{target}>; rel=\"next\"")).unwrap(),
        );
        let (client, fixture) = client(vec![response]);
        let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
        assert!(!pulls.completion.complete, "{target}");
        assert_eq!(fixture.paths().len(), 1);
    }
}

#[tokio::test]
async fn incomplete_check_count_is_explicit_and_unknown_status_survives() {
    let (client, _) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(
            200,
            json!({"total_count": 2, "check_runs": [check(10, SHA, "future_state", Some("success"))]}),
        ),
        response(200, json!([])),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert!(!checks.check_runs.completion.complete);
    assert_eq!(checks.check_runs.items[0].status_label(), "unknown status");
    assert!(checks.statuses.completion.complete);
    assert!(checks.statuses.items.is_empty());
}

#[tokio::test]
async fn response_body_failure_is_distinct_from_http_failure() {
    for (status, expected) in [
        (200, ApiErrorKind::Body),
        (401, ApiErrorKind::Authentication),
        (403, ApiErrorKind::Forbidden),
        (429, ApiErrorKind::RateLimit),
    ] {
        let (client, fixture) = client(vec![Response {
            status,
            headers: HeaderMap::new(),
            body: Err(error(ApiErrorKind::Body)),
        }]);
        let error = client.repository(&id("org/repo")).await.unwrap_err();
        assert_eq!(error.kind, expected);
        assert_eq!(fixture.paths().len(), 1);
    }
}

#[tokio::test]
async fn invalid_and_oversize_success_bodies_are_not_silent_empty_results() {
    for (body, expected) in [
        (
            b"{bad secret response".to_vec(),
            ApiErrorKind::InvalidResponse,
        ),
        (vec![b'a'; MAX_BODY_BYTES + 1], ApiErrorKind::BodyTooLarge),
    ] {
        let (client, _) = client(vec![Response {
            status: 200,
            headers: HeaderMap::new(),
            body: Ok(body),
        }]);
        let error = client.repository(&id("org/repo")).await.unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(!error.to_string().contains("secret response"));
    }
}

#[tokio::test]
async fn redirects_are_rejected_without_follow_up_or_credential_forwarding() {
    let mut response = response(302, json!({"message":"secret"}));
    response.headers.insert(
        "location",
        HeaderValue::from_static("https://evil.example/token"),
    );
    let (client, fixture) = client(vec![response]);
    assert_eq!(
        client.repository(&id("org/repo")).await.unwrap_err().kind,
        ApiErrorKind::Redirect
    );
    assert_eq!(fixture.paths().len(), 1);
}

#[test]
fn rate_limits_are_distinct_and_only_safe_numeric_guidance_survives() {
    let mut limited = response(403, json!({"message":"credential content must not escape"}));
    limited
        .headers
        .insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
    limited
        .headers
        .insert("retry-after", HeaderValue::from_static("60"));
    limited
        .headers
        .insert("x-ratelimit-reset", HeaderValue::from_static("1791723600"));
    let error = response_error(&limited);
    assert_eq!(error.kind, ApiErrorKind::RateLimit);
    assert_eq!(error.retry_after_secs, Some(60));
    assert_eq!(error.rate_limit_reset, Some(1791723600));
    assert!(!error.to_string().contains("credential"));
    assert!(error.to_string().contains("60s"));
    assert!(error.to_string().contains("1791723600"));
    let secondary = response_error(&response(
        403,
        json!({"message":"You exceeded a secondary rate limit."}),
    ));
    assert_eq!(secondary.kind, ApiErrorKind::RateLimit);
    let forbidden = response_error(&response(403, json!({"message":"Resource not accessible"})));
    assert_eq!(forbidden.kind, ApiErrorKind::Forbidden);
    let missing = response_error(&response(404, json!({"message":"Not Found"})));
    assert_eq!(missing.kind, ApiErrorKind::NotFound);
}

#[tokio::test]
async fn network_failure_is_safe_and_never_retried() {
    let fixture = FixtureTransport::new(vec![]);
    fixture
        .responses
        .lock()
        .unwrap()
        .push_back(Err(error(ApiErrorKind::Network)));
    let client = Client::with_transport(&[id("org/repo")], "", fixture.clone()).unwrap();
    assert_eq!(
        client.repository(&id("org/repo")).await.unwrap_err().kind,
        ApiErrorKind::Network
    );
    assert_eq!(fixture.paths().len(), 1);
}

#[tokio::test]
async fn rate_limited_later_check_page_stops_status_reads_with_numeric_guidance() {
    let path = format!("/repos/org/repo/commits/{SHA}/check-runs");
    let mut limited = response(403, json!({"message": "secondary rate limit"}));
    limited
        .headers
        .insert("retry-after", HeaderValue::from_static("60"));
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        page(
            json!({"total_count": 2, "check_runs": [check(10, SHA, "completed", Some("success"))]}),
            &path,
            2,
        ),
        limited,
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(checks.check_runs.items.len(), 1);
    assert_eq!(
        checks.check_runs.completion.error,
        Some(ApiErrorKind::RateLimit)
    );
    assert!(checks.check_runs.completion.reason.unwrap().contains("60s"));
    assert!(!checks.statuses.completion.complete);
    assert_eq!(fixture.paths().len(), 3);
}

#[tokio::test]
async fn oversized_later_page_keeps_committed_results_and_error_evidence() {
    let (client, _) = client(vec![
        page(
            json!([pull(7, Some("org/repo"))]),
            "/repos/org/repo/pulls",
            2,
        ),
        response(
            200,
            Value::Array((100..201).map(|n| pull(n, Some("org/repo"))).collect()),
        ),
    ]);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), 1);
    assert_eq!(pulls.completion.pages, 1);
    assert_eq!(pulls.completion.error, Some(ApiErrorKind::InvalidResponse));
}

#[tokio::test]
async fn full_page_without_next_link_is_endpoint_complete_per_github_contract() {
    let rows = (1..=100)
        .map(|number| pull(number, Some("org/repo")))
        .collect();
    let (client, fixture) = client(vec![response(200, Value::Array(rows))]);
    let pulls = client.open_pulls(&id("org/repo")).await.unwrap();
    assert_eq!(pulls.items.len(), 100);
    assert!(pulls.completion.complete);
    assert_eq!(fixture.paths().len(), 1);
}

#[test]
fn invalid_tokens_and_excess_configuration_are_rejected_before_reads() {
    let fixture = FixtureTransport::new(vec![]);
    let result =
        Client::with_transport(&[id("org/repo")], "bad token with spaces", fixture.clone());
    assert!(matches!(
        result,
        Err(ApiError {
            kind: ApiErrorKind::InvalidTokenReference,
            ..
        })
    ));
    let repositories = (0..=MAX_REPOSITORIES)
        .map(|n| id(&format!("org/repo{n}")))
        .collect::<Vec<_>>();
    let result = Client::with_transport(&repositories, "", fixture.clone());
    assert!(matches!(
        result,
        Err(ApiError {
            kind: ApiErrorKind::InvalidTarget,
            ..
        })
    ));
    assert!(fixture.paths().is_empty());
}

#[tokio::test]
async fn legacy_contexts_are_case_insensitive_and_newest_spelling_is_kept() {
    let (client, _) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(200, json!({"total_count": 0, "check_runs": []})),
        response(
            200,
            json!([
                status(3, "CI/Build", "pending", "org/repo"),
                status(2, "ci/build", "success", "org/repo"),
                status(1, "other", "failure", "org/repo")
            ]),
        ),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(checks.statuses.items.len(), 2);
    assert_eq!(checks.statuses.items[0].context, "CI/Build");
    assert_eq!(checks.statuses.items[0].state, "pending");
    assert!(checks.statuses.completion.complete);
}

#[tokio::test]
async fn repository_metadata_requires_explicit_visibility_archive_and_default_branch() {
    for field in ["private", "archived", "default_branch"] {
        for null in [false, true] {
            let mut raw = repository("org/repo");
            if null {
                raw[field] = Value::Null;
            } else {
                raw.as_object_mut().unwrap().remove(field);
            }
            let (client, fixture) = client(vec![response(200, raw)]);
            let error = client.repository(&id("org/repo")).await.unwrap_err();
            assert_eq!(
                error.kind,
                ApiErrorKind::InvalidResponse,
                "{field}, null={null}"
            );
            assert_eq!(
                error.to_string(),
                "GitHub response did not match the requested resource"
            );
            assert_eq!(fixture.paths().len(), 1);
        }
    }
    let mut raw = repository("org/repo");
    raw["default_branch"] = json!("");
    let (client, _) = client(vec![response(200, raw)]);
    assert_eq!(
        client.repository(&id("org/repo")).await.unwrap_err().kind,
        ApiErrorKind::InvalidResponse
    );
}

#[tokio::test]
async fn nested_repository_identity_does_not_require_unrequested_metadata() {
    let mut raw = pull(7, Some("contributor/fork"));
    for branch in ["base", "head"] {
        for field in ["private", "archived", "default_branch"] {
            raw[branch]["repo"].as_object_mut().unwrap().remove(field);
        }
    }
    let (client, fixture) = client(vec![response(200, raw)]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    assert_eq!(pull.head_repository, Some(id("contributor/fork")));
    assert_eq!(pull.repository, id("org/repo"));
    assert_eq!(fixture.paths().len(), 1);
}

#[tokio::test]
async fn missing_draft_status_is_rejected_instead_of_inventing_non_draft() {
    let mut raw = pull(7, Some("org/repo"));
    raw.as_object_mut().unwrap().remove("draft");
    let (detail_client, _) = client(vec![response(200, raw.clone())]);
    let error = detail_client.pull(&id("org/repo"), 7).await.unwrap_err();
    assert_eq!(error.kind, ApiErrorKind::InvalidResponse);
    assert_eq!(
        error.to_string(),
        "GitHub response did not match the requested resource"
    );
    let (list_client, _) = client(vec![response(200, json!([raw]))]);
    assert_eq!(
        list_client
            .open_pulls(&id("org/repo"))
            .await
            .unwrap_err()
            .kind,
        ApiErrorKind::InvalidResponse
    );
}

#[tokio::test]
async fn same_repository_head_is_read_once_and_provenance_is_explicit() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("org/repo"))),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("success"))]}),
        ),
        response(200, json!([status(20, "ci", "success", "org/repo")])),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(fixture.paths().len(), 3);
    assert_eq!(checks.check_runs.completion.pages, 1);
    assert_eq!(checks.statuses.completion.pages, 1);
    assert_eq!(checks.check_runs.items[0].repository, id("org/repo"));
    assert_eq!(checks.statuses.items[0].repository, id("org/repo"));
}

#[tokio::test]
async fn source_rate_limit_or_auth_retains_base_rows_and_skips_source_statuses() {
    for (code, kind) in [
        (429, ApiErrorKind::RateLimit),
        (401, ApiErrorKind::Authentication),
    ] {
        let (client, fixture) = client(vec![
            response(200, pull(7, Some("contributor/fork"))),
            response(
                200,
                json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("success"))]}),
            ),
            response(200, json!([status(20, "ci", "success", "org/repo")])),
            response(code, json!({"message": "untrusted private error content"})),
        ]);
        let pull = client.pull(&id("org/repo"), 7).await.unwrap();
        let checks = client.checks(&pull).await.unwrap();
        assert_eq!(fixture.paths().len(), 4);
        assert_eq!(checks.check_runs.items.len(), 1);
        assert_eq!(checks.statuses.items.len(), 1);
        assert!(!checks.check_runs.completion.complete);
        assert!(!checks.statuses.completion.complete);
        assert_eq!(checks.check_runs.completion.error, Some(kind));
        assert!(
            checks
                .statuses
                .completion
                .reason
                .unwrap()
                .contains("not requested")
        );
        assert!(
            checks
                .errors
                .iter()
                .all(|error| !error.contains("untrusted"))
        );
    }
}

#[tokio::test]
async fn source_permission_failure_keeps_independent_base_and_source_statuses() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("contributor/fork"))),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("failure"))]}),
        ),
        response(200, json!([status(20, "ci", "failure", "org/repo")])),
        response(403, json!({"message": "Resource not accessible"})),
        response(
            200,
            json!([status(21, "ci", "success", "contributor/fork")]),
        ),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(fixture.paths().len(), 5);
    assert!(!checks.check_runs.completion.complete);
    assert_eq!(
        checks.check_runs.completion.error,
        Some(ApiErrorKind::Forbidden)
    );
    assert_eq!(checks.check_runs.items.len(), 1);
    assert!(checks.statuses.completion.complete);
    assert_eq!(checks.statuses.items.len(), 2);
    assert_eq!(checks.statuses.items[1].repository, id("contributor/fork"));
}

#[tokio::test]
async fn base_status_rate_limit_stops_all_source_requests_with_base_check_preserved() {
    let (client, fixture) = client(vec![
        response(200, pull(7, Some("contributor/fork"))),
        response(
            200,
            json!({"total_count": 1, "check_runs": [check(10, SHA, "completed", Some("success"))]}),
        ),
        response(429, json!({"message": "rate limited"})),
    ]);
    let pull = client.pull(&id("org/repo"), 7).await.unwrap();
    let checks = client.checks(&pull).await.unwrap();
    assert_eq!(fixture.paths().len(), 3);
    assert_eq!(checks.check_runs.items.len(), 1);
    assert!(!checks.check_runs.completion.complete);
    assert!(!checks.statuses.completion.complete);
    assert_eq!(
        checks.statuses.completion.error,
        Some(ApiErrorKind::RateLimit)
    );
    assert!(
        checks
            .check_runs
            .completion
            .reason
            .unwrap()
            .contains("source repository")
    );
}
