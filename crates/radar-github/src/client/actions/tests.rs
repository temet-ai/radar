use super::*;
use serde_json::{Value, json};
use std::collections::VecDeque;

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    responses: Mutex<VecDeque<Response>>,
    requests: Mutex<Vec<String>>,
    /// The `If-None-Match` of each request, in order.
    asked: Mutex<Vec<Option<String>>>,
}

impl Transport for Fixture {
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            assert!(request.authorization.is_none());
            assert!(request.graphql.is_none());
            self.asked.lock().unwrap().push(
                request
                    .headers
                    .get(IF_NONE_MATCH)
                    .map(|v| v.to_str().unwrap().to_string()),
            );
            self.requests.lock().unwrap().push(request.url);
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("extra read"))
        })
    }
}

impl Client {
    /// Every page up to the cap, as the list reader pages any endpoint: the
    /// tab itself asks for one bounded page (`workflow_runs_if`).
    async fn workflow_runs(
        &self,
        repository: &RepositoryId,
    ) -> Result<Collection<WorkflowRun>, ApiError> {
        let repository = self.allowed(repository)?;
        let path = format!("/repos/{}/actions/runs", repository.full_name());
        self.collect(&path, "", Some("workflow_runs"), |raw: RawWorkflowRun| {
            let run = raw.checked(&Endpoint::default(), &repository, None, None)?;
            Ok((run.id, run))
        })
        .await
    }
}

fn id(name: &str) -> RepositoryId {
    RepositoryId::parse(name).unwrap()
}

fn fixture(responses: Vec<Response>) -> (Client, Arc<Fixture>) {
    let transport = Arc::new(Fixture {
        responses: Mutex::new(responses.into()),
        requests: Mutex::new(vec![]),
        asked: Mutex::new(vec![]),
    });
    (
        Client::with_transport(&[id("org/repo")], "", transport.clone()).unwrap(),
        transport,
    )
}

fn response(status: u16, body: Value) -> Response {
    Response {
        status,
        headers: HeaderMap::new(),
        body: Ok(serde_json::to_vec(&body).unwrap()),
    }
}

fn page(body: Value, endpoint: &str, next: u32) -> Response {
    let mut response = response(200, body);
    response.headers.insert(
        LINK,
        HeaderValue::from_str(&format!(
            "<https://api.github.com/repos/org/repo/{endpoint}?page={next}>; rel=\"next\""
        ))
        .unwrap(),
    );
    response
}

fn repository(name: &str) -> Value {
    let (owner, short) = name.split_once('/').unwrap();
    json!({"name":short,"owner":{"login":owner},"full_name":name,
        "html_url":format!("https://github.com/{name}")})
}

fn run(number: u64, attempt: u64) -> Value {
    json!({
        "id":number,"workflow_id":9,"run_number":42,"run_attempt":attempt,
        "name":"Build","display_title":"Check the change","path":".github/workflows/build.yml",
        "event":"pull_request","status":"completed","conclusion":"failure",
        "head_sha":SHA,"head_branch":"topic","head_repository":repository("alice/fork"),
        "repository":repository("org/repo"),
        "url":format!("https://api.github.com/repos/org/repo/actions/runs/{number}"),
        "html_url":format!("https://github.com/org/repo/actions/runs/{number}"),
        "created_at":"2026-10-11T12:00:00Z","updated_at":"2026-10-11T13:00:00Z",
        "run_started_at":"2026-10-11T12:59:00Z","pull_requests":[]
    })
}

fn step(number: u64) -> Value {
    json!({"number":number,"name":"Test","status":"completed","conclusion":"failure",
        "started_at":"2026-10-11T12:59:03Z","completed_at":"2026-10-11T12:59:09Z"})
}

fn job(number: u64, attempt: u64) -> Value {
    json!({
        "id":number,"run_id":501,"run_attempt":attempt,"head_sha":SHA,
        "url":format!("https://api.github.com/repos/org/repo/actions/jobs/{number}"),
        "run_url":"https://api.github.com/repos/org/repo/actions/runs/501",
        "html_url":format!("https://github.com/org/repo/actions/runs/501/job/{number}"),
        "name":"linux","status":"completed","conclusion":"failure",
        "started_at":"2026-10-11T12:59:01Z","completed_at":"2026-10-11T12:59:10Z",
        "steps":[step(1),step(3)]
    })
}

fn association(base: &str, number: u64, head: Option<&str>, sha: &str) -> Value {
    let repo = |name: &str| {
        json!({"id":7,"name":name.split_once('/').unwrap().1,
            "url":format!("https://api.github.com/repos/{name}")})
    };
    json!({"number":number,"url":format!("https://api.github.com/repos/{base}/pulls/{number}"),
        "base":{"sha":"b".repeat(40),"repo":repo(base)},
        "head":{"sha":sha,"repo":head.map(repo)}})
}

fn pull() -> PullRequest {
    PullRequest {
        repository: id("org/repo"),
        number: 17,
        title: "Change".into(),
        body: None,
        author: "alice".into(),
        state: "open".into(),
        draft: false,
        head_sha: SHA.into(),
        head_repository: Some(id("alice/fork")),
        head_branch: "topic".into(),
        base_branch: "main".into(),
        updated_at: "2026-10-11T12:00:00Z".into(),
        merged: false,
    }
}

#[tokio::test]
async fn actions_list_detail_and_jobs_use_configured_pinned_get_paths() {
    let (client, transport) = fixture(vec![
        response(200, json!({"total_count":1,"workflow_runs":[run(501,2)]})),
        response(200, run(501, 2)),
        response(200, json!({"total_count":1,"jobs":[job(701,2)]})),
    ]);
    let runs = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert!(runs.completion.complete);
    assert_eq!(runs.items[0].run_attempt, 2);
    assert_eq!(
        client.workflow_jobs(&runs.items[0]).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    let detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    let jobs = client.workflow_jobs(&detail).await.unwrap();
    assert!(jobs.completion.complete);
    assert_eq!(jobs.items[0].run_attempt, 2);
    assert_eq!(jobs.items[0].steps[1].number, 3);
    assert_eq!(
        detail.web_url(),
        "https://github.com/org/repo/actions/runs/501/attempts/2"
    );
    assert_eq!(
        jobs.items[0].web_url(),
        "https://github.com/org/repo/actions/runs/501/job/701"
    );
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![
            "https://api.github.com/repos/org/repo/actions/runs?per_page=100&page=1",
            "https://api.github.com/repos/org/repo/actions/runs/501/attempts/2",
            "https://api.github.com/repos/org/repo/actions/runs/501/attempts/2/jobs?per_page=100&page=1",
        ]
    );
}

#[tokio::test]
async fn actions_forged_or_replaced_attempt_proof_cannot_read_jobs() {
    let (client, transport) = fixture(vec![response(200, run(501, 2)), response(200, run(501, 3))]);
    let first = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    for change in 0..4 {
        let mut forged = first.clone();
        match change {
            0 => forged.repository = id("other/repo"),
            1 => forged.id += 1,
            2 => forged.run_attempt += 1,
            _ => forged.head_sha = "b".repeat(40),
        }
        assert_eq!(
            client.workflow_jobs(&forged).await.unwrap_err().kind,
            ApiErrorKind::InvalidTarget
        );
    }
    client
        .clone()
        .workflow_run(&id("org/repo"), 501)
        .await
        .unwrap();
    assert_eq!(
        client.workflow_jobs(&first).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn actions_failed_detail_and_invalid_targets_invalidate_old_proof() {
    let (client, transport) = fixture(vec![
        response(200, run(501, 2)),
        response(404, json!({"message":"private"})),
    ]);
    let old = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    assert_eq!(
        client
            .workflow_run_attempt(&id("org/repo"), 501, 2)
            .await
            .unwrap_err()
            .kind,
        ApiErrorKind::NotFound
    );
    assert_eq!(
        client.workflow_jobs(&old).await.unwrap_err().kind,
        ApiErrorKind::InvalidTarget
    );
    for (repository, number, attempt) in [
        (id("org/repo"), 0, 1),
        (id("org/repo"), 501, 0),
        (id("other/repo"), 501, 1),
    ] {
        assert_eq!(
            client
                .workflow_run_attempt(&repository, number, attempt)
                .await
                .unwrap_err()
                .kind,
            ApiErrorKind::InvalidTarget
        );
    }
    assert_eq!(
        client
            .workflow_runs(&id("other/repo"))
            .await
            .unwrap_err()
            .kind,
        ApiErrorKind::InvalidTarget
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn actions_wrong_run_attempt_repository_and_urls_are_rejected() {
    for change in 0..8 {
        let mut raw = run(501, 2);
        match change {
            0 => raw["id"] = json!(502),
            1 => raw["run_attempt"] = json!(3),
            2 => raw["repository"] = repository("other/repo"),
            3 => raw["url"] = json!("https://elsewhere.test/run/501"),
            4 => raw["html_url"] = json!("https://github.com/org/repo/actions/runs/501/attempts/3"),
            5 => raw["head_sha"] = json!("not-a-sha"),
            6 => raw["workflow_id"] = json!(0),
            _ => {
                raw.as_object_mut().unwrap().remove("run_attempt");
            }
        }
        let (client, transport) = fixture(vec![response(200, raw)]);
        assert_eq!(
            client
                .workflow_run_attempt(&id("org/repo"), 501, 2)
                .await
                .unwrap_err()
                .kind,
            ApiErrorKind::InvalidResponse,
            "change {change}"
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn actions_jobs_validate_run_attempt_sha_repository_and_unique_steps() {
    for change in 0..7 {
        let mut raw = job(701, 2);
        match change {
            0 => raw["run_id"] = json!(502),
            1 => raw["run_attempt"] = json!(1),
            2 => raw["head_sha"] = json!("b".repeat(40)),
            3 => raw["url"] = json!("https://api.github.com/repos/other/repo/actions/jobs/701"),
            4 => {
                raw["run_url"] =
                    json!("https://api.github.com/repos/org/repo/actions/runs/501/attempts/3")
            }
            5 => raw["steps"] = json!([step(1), step(1)]),
            _ => raw["steps"] = json!([step(0)]),
        }
        let (client, _) = fixture(vec![
            response(200, run(501, 2)),
            response(200, json!({"total_count":1,"jobs":[raw]})),
        ]);
        let detail = client
            .workflow_run_attempt(&id("org/repo"), 501, 2)
            .await
            .unwrap();
        assert_eq!(
            client.workflow_jobs(&detail).await.unwrap_err().kind,
            ApiErrorKind::InvalidResponse,
            "change {change}"
        );
    }
}

#[tokio::test]
async fn actions_optional_attempt_fields_and_unknown_states_remain_truthful() {
    let mut raw_run = run(501, 2);
    raw_run["status"] = Value::Null;
    raw_run["conclusion"] = Value::Null;
    raw_run["head_branch"] = Value::Null;
    raw_run["head_repository"] = Value::Null;
    let mut raw_job = job(701, 2);
    raw_job.as_object_mut().unwrap().remove("run_attempt");
    raw_job["status"] = json!("future");
    raw_job["started_at"] = Value::Null;
    raw_job["completed_at"] = Value::Null;
    raw_job["steps"][0]["status"] = json!("completed");
    raw_job["steps"][0]["conclusion"] = Value::Null;
    let (client, _) = fixture(vec![
        response(200, raw_run),
        response(200, json!({"total_count":1,"jobs":[raw_job]})),
    ]);
    let detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    assert_eq!(detail.status_label(), "unknown status");
    assert!(detail.head_branch.is_none());
    assert!(detail.head_repository.is_none());
    let jobs = client.workflow_jobs(&detail).await.unwrap();
    let job = &jobs.items[0];
    assert_eq!(job.run_attempt, 2);
    assert_eq!(job.status_label(), "unknown status");
    assert!(job.started_at.is_none() && job.completed_at.is_none());
    assert_eq!(job.steps[0].status_label(), "unknown conclusion");
    assert_eq!(
        job.steps[0].started_at.as_deref(),
        Some("2026-10-11T12:59:03Z")
    );
}

#[tokio::test]
async fn actions_pr_associations_preserve_foreign_bases_and_merge_sha_distinction() {
    let mut raw = run(501, 2);
    raw["head_sha"] = json!("c".repeat(40));
    raw["pull_requests"] = json!([
        association("another/fork", 17, Some("alice/fork"), SHA),
        association("org/repo", 17, Some("alice/fork"), SHA),
    ]);
    let (client, _) = fixture(vec![response(200, raw)]);
    let detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    assert_eq!(detail.pull_requests[0].repository, id("another/fork"));
    assert_ne!(detail.pull_requests[1].head_sha, detail.head_sha);
    assert_eq!(detail.pull_match(&pull()), Some(RunPullMatch::Associated));
    let mut only_foreign = detail.clone();
    only_foreign.pull_requests.remove(1);
    assert_eq!(only_foreign.pull_match(&pull()), None);
}

#[tokio::test]
async fn actions_sha_fallback_requires_source_identity_not_branch_or_number() {
    let (client, _) = fixture(vec![response(200, run(501, 2))]);
    let mut detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    assert_eq!(detail.pull_match(&pull()), Some(RunPullMatch::HeadSha));
    detail.head_repository = Some(id("other/fork"));
    assert_eq!(detail.pull_match(&pull()), None);
    detail.head_repository = None;
    let mut deleted_source = pull();
    deleted_source.head_repository = None;
    assert_eq!(detail.pull_match(&deleted_source), None);
}

#[tokio::test]
async fn actions_pr_association_url_and_minimal_repository_are_validated() {
    for change in 0..4 {
        let mut assoc = association("org/repo", 17, Some("alice/fork"), SHA);
        match change {
            0 => assoc["url"] = json!("https://api.github.com/repos/other/repo/pulls/17"),
            1 => assoc["base"]["repo"]["url"] = json!("https://elsewhere.test/org/repo"),
            2 => assoc["head"]["repo"]["name"] = json!("mismatch"),
            _ => assoc["head"]["sha"] = json!("invalid"),
        }
        let mut raw = run(501, 2);
        raw["pull_requests"] = json!([assoc]);
        let (client, _) = fixture(vec![response(200, raw)]);
        assert_eq!(
            client
                .workflow_run_attempt(&id("org/repo"), 501, 2)
                .await
                .unwrap_err()
                .kind,
            ApiErrorKind::InvalidResponse
        );
    }
}

#[tokio::test]
async fn actions_empty_activity_is_distinct_from_permission_or_missing_resource() {
    let (client, _) = fixture(vec![response(
        200,
        json!({"total_count":0,"workflow_runs":[]}),
    )]);
    let empty = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert!(empty.items.is_empty() && empty.completion.complete);
    for (status, kind) in [
        (401, ApiErrorKind::Authentication),
        (403, ApiErrorKind::Forbidden),
        (404, ApiErrorKind::NotFound),
        (429, ApiErrorKind::RateLimit),
    ] {
        let (client, transport) = fixture(vec![response(
            status,
            json!({"message":"not included in output"}),
        )]);
        assert_eq!(
            client
                .workflow_runs(&id("org/repo"))
                .await
                .unwrap_err()
                .kind,
            kind
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn actions_later_failed_page_keeps_activity_and_its_error() {
    let (client, _) = fixture(vec![
        page(
            json!({"total_count":2,"workflow_runs":[run(501,2)]}),
            "actions/runs",
            2,
        ),
        response(403, json!({"message":"denied"})),
    ]);
    let runs = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert_eq!(runs.items.len(), 1);
    assert!(!runs.completion.complete);
    assert_eq!(runs.completion.error, Some(ApiErrorKind::Forbidden));
    assert_eq!(runs.completion.pages, 1);
}

#[tokio::test]
async fn actions_changed_duplicate_runs_and_page_cap_are_explicitly_partial() {
    let (client, _) = fixture(vec![
        page(
            json!({"total_count":2,"workflow_runs":[run(501,2)]}),
            "actions/runs",
            2,
        ),
        response(200, json!({"total_count":2,"workflow_runs":[run(501,3)]})),
    ]);
    let runs = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert_eq!(runs.items.len(), 1);
    assert!(!runs.completion.complete);
    let (client, transport) = fixture(
        (1..=MAX_PAGES)
            .map(|number| {
                page(
                    json!({"total_count":50,"workflow_runs":[run(500+u64::from(number),1)]}),
                    "actions/runs",
                    number + 1,
                )
            })
            .collect(),
    );
    let runs = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert_eq!(runs.items.len(), MAX_PAGES as usize);
    assert_eq!(runs.completion.reason.as_deref(), Some(PAGE_CAP));
    assert_eq!(transport.requests.lock().unwrap().len(), MAX_PAGES as usize);
}

#[tokio::test]
async fn actions_rate_limited_jobs_keep_prior_page_and_numeric_retry_guidance() {
    let mut denied = response(403, json!({"message":"secondary rate limit"}));
    denied
        .headers
        .insert("retry-after", HeaderValue::from_static("90"));
    let (client, transport) = fixture(vec![
        response(200, run(501, 2)),
        page(
            json!({"total_count":2,"jobs":[job(701,2)]}),
            "actions/runs/501/attempts/2/jobs",
            2,
        ),
        denied,
    ]);
    let detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    let jobs = client.workflow_jobs(&detail).await.unwrap();
    assert_eq!(jobs.items.len(), 1);
    assert!(!jobs.completion.complete);
    assert_eq!(jobs.completion.error, Some(ApiErrorKind::RateLimit));
    assert!(
        jobs.completion
            .reason
            .as_deref()
            .unwrap()
            .contains("retry after 90s")
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn actions_hostile_continuation_is_never_followed() {
    let mut first = response(200, json!({"total_count":2,"workflow_runs":[run(501,2)]}));
    first.headers.insert(
        LINK,
        HeaderValue::from_static("<https://elsewhere.test/actions/runs?page=2>; rel=\"next\""),
    );
    let (client, transport) = fixture(vec![first]);
    let runs = client.workflow_runs(&id("org/repo")).await.unwrap();
    assert_eq!(runs.items.len(), 1);
    assert_eq!(runs.completion.reason.as_deref(), Some(INVALID_PAGINATION));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn actions_urls_allow_repository_case_but_not_encoded_or_ambiguous_targets() {
    let mut raw = run(501, 2);
    raw["repository"] = repository("Org/Repo");
    raw["url"] = json!("https://api.github.com/repos/Org/Repo/actions/runs/501");
    raw["html_url"] = json!("https://github.com/Org/Repo/actions/runs/501");
    raw["pull_requests"] = json!([association("Org/Repo", 17, Some("Alice/Fork"), SHA)]);
    let mut raw_job = job(701, 2);
    raw_job["url"] = json!("https://api.github.com/repos/Org/Repo/actions/jobs/701");
    raw_job["run_url"] = json!("https://api.github.com/repos/Org/Repo/actions/runs/501");
    let (client, _) = fixture(vec![
        response(200, raw),
        response(200, json!({"total_count":1,"jobs":[raw_job]})),
    ]);
    let detail = client
        .workflow_run_attempt(&id("org/repo"), 501, 2)
        .await
        .unwrap();
    assert_eq!(detail.pull_match(&pull()), Some(RunPullMatch::Associated));
    assert_eq!(client.workflow_jobs(&detail).await.unwrap().items.len(), 1);
    for url in [
        "https://api.github.com/repos/org/repo/actions/runs/501?token=secret",
        "https://api.github.com/repos/org/repo/actions/runs/501#fragment",
        "https://api.github.com/repos/org/re%70o/actions/runs/501",
        "https://api.github.com@elsewhere.test/repos/org/repo/actions/runs/501",
        "https://api.github.com/repos/org/repo/actions/runs/0501",
    ] {
        assert!(!valid_run_url(
            Some(&Endpoint::default()),
            url,
            &id("org/repo"),
            501,
            2
        ));
    }
}

#[test]
fn run_addresses_are_recognised_under_the_configured_api_only() {
    let ghe = Endpoint::parse("https://ghe.example/api/v3").unwrap();
    let run = format!("{}/repos/org/repo/actions/runs/501", ghe.rest());
    assert!(valid_run_url(Some(&ghe), &run, &id("org/repo"), 501, 1));
    assert!(!valid_run_url(
        Some(&Endpoint::default()),
        &run,
        &id("org/repo"),
        501,
        1
    ));
}

fn ask(branch: Option<&str>, per_page: usize) -> RunsAsk {
    RunsAsk {
        branch: branch.map(Into::into),
        per_page,
    }
}
fn tagged(mut response: Response, etag: &str) -> Response {
    response
        .headers
        .insert(ETAG, HeaderValue::from_str(etag).unwrap());
    response
}
fn etag(value: &str) -> Validator {
    Validator {
        etag: Some(value.into()),
        last_modified: None,
    }
}
const RUNS: &str = "https://api.github.com/repos/org/repo/actions/runs";

#[tokio::test]
async fn the_tab_asks_one_page_of_the_newest_runs_and_no_more() {
    // GitHub has more: the Link header names a page two the tab never asks for.
    let (client, transport) = fixture(vec![tagged(
        page(
            json!({"total_count":900,"workflow_runs":[run(501,1),run(500,1)]}),
            "actions/runs",
            2,
        ),
        "W/\"runs\"",
    )]);
    let Fresh::New(runs, tags) = client
        .workflow_runs_if(&id("org/repo"), &ask(None, 40), &[])
        .await
        .unwrap()
    else {
        panic!("a first read is new");
    };
    assert_eq!(runs.items.len(), 2);
    assert!(runs.completion.complete, "complete for what was asked");
    assert!(
        runs.completion
            .reason
            .as_deref()
            .unwrap()
            .contains("newest 2"),
        "and says it is the newest only: {:?}",
        runs.completion.reason
    );
    assert_eq!(tags, [etag("W/\"runs\"")], "the bounded page is tagged");
    assert_eq!(
        *transport.requests.lock().unwrap(),
        [format!("{RUNS}?per_page=40&page=1")],
        "one request, 40 rows, not five pages of 100"
    );
}

#[tokio::test]
async fn a_pull_requests_runs_are_asked_by_its_source_branch() {
    let (client, transport) = fixture(vec![response(
        200,
        json!({"total_count":1,"workflow_runs":[run(501,1)]}),
    )]);
    client
        .workflow_runs_if(&id("org/repo"), &ask(Some("feat/x&y"), 30), &[])
        .await
        .unwrap();
    assert_eq!(
        *transport.requests.lock().unwrap(),
        [format!("{RUNS}?branch=feat%2Fx%26y&per_page=30&page=1")],
        "the branch is encoded, never spliced into the query"
    );
}

#[tokio::test]
async fn an_unchanged_runs_ask_is_a_304_with_nothing_parsed() {
    let (client, transport) = fixture(vec![
        tagged(
            response(200, json!({"total_count":1,"workflow_runs":[run(501,1)]})),
            "\"a\"",
        ),
        Response {
            status: 304,
            headers: HeaderMap::new(),
            body: Ok(b"<not json>".to_vec()),
        },
    ]);
    let Fresh::New(_, tags) = client
        .workflow_runs_if(&id("org/repo"), &ask(None, 50), &[])
        .await
        .unwrap()
    else {
        panic!("new");
    };
    assert_eq!(
        client
            .workflow_runs_if(&id("org/repo"), &ask(None, 50), &tags)
            .await
            .unwrap(),
        Fresh::Same
    );
    assert_eq!(
        *transport.asked.lock().unwrap(),
        [None, Some("\"a\"".into())]
    );
    assert_eq!(client.traffic().not_modified, 1);
}

#[tokio::test]
async fn runs_asks_outside_the_page_or_the_branch_rules_never_reach_the_transport() {
    let (client, transport) = fixture(vec![]);
    for bad in [
        ask(None, 0),
        ask(None, 101),
        ask(Some(""), 50),
        ask(Some("a\nb"), 50),
    ] {
        assert_eq!(
            client
                .workflow_runs_if(&id("org/repo"), &bad, &[])
                .await
                .unwrap_err()
                .kind,
            ApiErrorKind::InvalidTarget
        );
    }
    assert!(transport.requests.lock().unwrap().is_empty());
}
