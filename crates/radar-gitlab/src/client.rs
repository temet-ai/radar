//! GitLab over GraphQL: per tenancy (a group or a single repo), its open
//! merge requests with pipeline, approvals and diff size, the merged and
//! closed ones a view asks for, or only what changed since the last sync;
//! plus its projects.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backoff::{Headers, Pacing, Trouble};
use crate::model::{
    Comment, CommitInfo, DiffRefs, Discussion, FileDiff, FileStat, Job, JobInfo, MainCommit,
    MergeSettings, Mr, MrDetail, MrState, Note, Person, Pipeline, PipelineDetail, PipelineRun,
    MainRef, Project, Stage, StageJobs, TracePart,
};

mod checks;
pub(super) use checks::CheckRead;

/// How much of a job's log is asked for at a time, from the end: a failed
/// job's first error is nearly always in its last part.
pub const TRACE_PART: u64 = 256 * 1024;

/// What radar holds for one tenancy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupData {
    /// Open ones, and the merged and closed ones fetched for views that ask.
    pub mrs: Vec<Mr>,
    pub projects: Vec<Project>,
    /// The tenancy is a single repo, not a group.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repo: bool,
    /// Merged and closed merge requests are held back to this time (RFC
    /// 3339); `None` when none were asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_since: Option<String>,
}

/// GitLab turned the token down (HTTP 401). Kept as its own error so a
/// caller can ask for a fresh token once and try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected;

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitLab rejected the token (401)")
    }
}

impl std::error::Error for Rejected {}

/// Whether `e` is GitLab turning the token down.
pub fn is_rejected(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<Rejected>())
}

/// What one sync of a tenancy asks GitLab for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Want {
    /// `None`: every open merge request (a full sync). `Some(t)`: only those
    /// updated since `t`, in any state, so ones that were merged or closed
    /// are seen leaving.
    pub changed_since: Option<String>,
    /// On a full sync, also the merged and closed ones updated since then.
    pub ended_since: Option<String>,
    /// Fetch the projects too.
    pub projects: bool,
    /// With `projects`: `None` fetches every project in full; `Some` asks
    /// each project's last activity first (cheap) and fetches in full only
    /// the ones that need it.
    pub projects_check: Option<ProjectsCheck>,
    /// Open merge requests to look at again although they did not change
    /// (a running pipeline changes without touching the merge request):
    /// (project path, iid).
    pub recheck: Vec<(String, u64)>,
    /// Open merge requests GitLab has not checked (`UNCHECKED`): it works out
    /// whether one can merge only when asked, so they are asked for with
    /// `with_merge_status_recheck` (a read) before the recheck reads them.
    pub nudge: Vec<(String, u64)>,
    /// What the path is, when known: `Some(true)` a repo, `Some(false)` a group.
    pub repo: Option<bool>,
}

/// Most merge requests asked to be checked in one sync.
pub const MOST_NUDGES: usize = 10;

/// What radar holds of a tenancy's projects, so a sync fetches in full only
/// the ones that changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectsCheck {
    /// Each held project's `lastActivityAt`, by path. A project whose
    /// activity moved, or that is not here (new), is fetched in full.
    pub seen: BTreeMap<String, String>,
    /// Projects fetched in full whatever their activity says, that have no
    /// reference to pinpoint them with: main's pipeline is running (its
    /// end does not move `lastActivityAt`), or they were active within the
    /// hour (GitLab moves `lastActivityAt` at most once an hour, so a second
    /// push in that hour does not show).
    pub again: BTreeSet<String>,
    /// Each held project's reference ([`Project::pin`]: branch, head commit,
    /// newest pipeline on it), by path. A project looked at that has one is
    /// asked only for these and fetched in full only if they differ.
    pub pins: BTreeMap<String, MainRef>,
    /// Projects with a reference that need looking at for the same reasons
    /// (a pipeline running, active within the hour): asked their reference
    /// whatever their activity says.
    pub look: BTreeSet<String>,
}

/// What a projects check brought back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectsSeen {
    /// Every unarchived project there is now: (path, `lastActivityAt`).
    pub present: Vec<(String, String)>,
    /// The ones fetched in full.
    pub fresh: Vec<Project>,
}

/// What a sync brought back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Synced {
    /// Who the token belongs to.
    pub me: String,
    pub repo: bool,
    pub mrs: Vec<Mr>,
    /// A bounded MR query stopped while GitLab advertised another page.
    /// Transient evidence only: the provider cache remains unchanged.
    pub mrs_truncated: bool,
    /// Optional merge-status recheck dispatches, not proof GitLab recalculated.
    pub nudges: Nudges,
    /// `None` when they were not asked for, or only checked.
    pub projects: Option<Vec<Project>>,
    /// Open merge requests asked for again ([`Want::recheck`]): only their
    /// pipeline and merge-status fields ([`RECHECK_FIELDS`]) are read, the
    /// rest left at their defaults for [`crate::sync::merge`] to keep.
    pub rechecked: Vec<Mr>,
    /// A projects check ([`Want::projects_check`]).
    pub projects_seen: Option<ProjectsSeen>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Nudges {
    pub attempted: usize,
    pub errors: usize,
    /// Targets beyond the existing per-sync dispatch bound.
    pub deferred: usize,
}

#[derive(Default)]
struct MrBatch {
    mrs: Vec<Mr>,
    truncated: bool,
}

/// At most this many pages of merged or closed merge requests, or of
/// changes, per sync: enough for weeks of a busy group, and bounded.
const MAX_PAGES: usize = 4;

/// A page of merge requests (GitLab prices a query by its page size).
const PAGE: usize = 50;

/// Merge requests a recheck asks for in one query, each by its project and
/// iid ([`recheck_query`]). Measured on gitlab.com: 2 cost 49 of the 250 a
/// query may, 8 cost 187. Asked by iids across a group instead (as before),
/// GitLab found each iid in every project of the group: 184 for 2.
const RECHECK_BATCH: usize = 8;

/// Projects fetched in full in one query, each by its path
/// ([`projects_query`]). Measured on gitlab.com: 2 cost 65, 6 cost 189 (213 once
/// the commit's and pipelines' ids and times were asked too).
const PROJECTS_BATCH: usize = 6;

/// Projects a pin query asks for at once ([`pins_query`]). Measured on
/// gitlab.com: 1 cost 28 of the 250 a query may, 3 cost 78 (25 each, against
/// 31 for a project in full), so 8 cost about 200.
const PINS_BATCH: usize = 8;

/// What a recheck reads of a merge request: what a running pipeline or
/// GitLab's own check changes without touching its update time. Holds
/// [`CHECKS`] whole, so an older server is asked again without it.
pub(crate) const RECHECK_FIELDS: &str = "detailedMergeStatus conflicts shouldBeRebased mergeabilityChecks { identifier status } headPipeline { status path duration stages { nodes { name status } } }";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

/// A request, its path relative to the server (`/api/graphql`,
/// `/api/v4/projects/…`).
#[derive(Debug, Clone, PartialEq)]
pub struct Req {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
    /// A `Range` header (a part of a job's log).
    pub range: Option<String>,
}

impl Req {
    pub fn new(method: Method, path: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            method,
            path: path.into(),
            body,
            range: None,
        }
    }
}

/// An answer: its status, its `Content-Range`, its body, and what its
/// headers said about pacing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resp {
    pub status: u16,
    pub content_range: Option<String>,
    pub body: Vec<u8>,
    pub pacing: Pacing,
}

pub type Sent<'a> = Pin<Box<dyn Future<Output = Result<Resp, String>> + Send + 'a>>;

/// What carries requests: HTTP, or recorded replies in tests.
pub trait Transport: Send + Sync {
    fn send(&self, req: &Req) -> Sent<'_>;
}

struct Http {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Transport for Http {
    fn send(&self, req: &Req) -> Sent<'_> {
        let url = format!("{}{}", self.base, req.path);
        let req = req.clone();
        Box::pin(async move {
            let mut r = match req.method {
                Method::Get => self.http.get(&url),
                Method::Post => self.http.post(&url),
                Method::Put => self.http.put(&url),
            }
            .bearer_auth(&self.token);
            if let Some(b) = &req.body {
                r = r.json(b);
            }
            if let Some(range) = &req.range {
                r = r.header(reqwest::header::RANGE, range);
            }
            let resp = r.send().await.map_err(|e| format!("GitLab unreachable: {e}"))?;
            let status = resp.status().as_u16();
            let content_range = resp
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok());
            let pacing = Pacing::read(
                &Headers {
                    retry_after: header("retry-after"),
                    limit: header("ratelimit-limit"),
                    remaining: header("ratelimit-remaining"),
                    reset: header("ratelimit-reset"),
                },
                radar_core::time::now_secs(),
            );
            let body = match resp.bytes().await {
                Ok(body) => body.to_vec(),
                // Receiving a successful status is not proof the body arrived.
                // Keep this error free of URLs, credentials and partial content.
                Err(_) if (200..300).contains(&status) => {
                    return Err("GitLab response body could not be read.".into());
                }
                // Preserve status-based rejection/refusal and the legacy 416
                // trace default when an error response cannot be read.
                Err(_) => Vec::new(),
            };
            Ok(Resp {
                status,
                content_range,
                body,
                pacing,
            })
        })
    }
}

#[derive(Clone)]
pub struct Client {
    t: Arc<dyn Transport>,
    /// The server, without a trailing slash.
    base: String,
    /// What the latest answer said about pacing, shared by the clones.
    pacing: Arc<Mutex<Pacing>>,
}

/// One HTTP client per server, kept for the life of the process: every
/// request after the first reuses its connections (and their TLS session).
pub(crate) fn shared_http(base: &str) -> reqwest::Result<reqwest::Client> {
    static POOL: OnceLock<Mutex<std::collections::HashMap<String, reqwest::Client>>> =
        OnceLock::new();
    let mut pool = POOL
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = base.trim_end_matches('/');
    if let Some(http) = pool.get(key) {
        return Ok(http.clone());
    }
    let http = reqwest::Client::builder()
        .user_agent("radar")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    pool.insert(key.to_string(), http.clone());
    Ok(http)
}

/// GitLab's own list of what stands between a merge request and merging
/// (newer servers only; [`Client::query`] asks again without it).
const CHECKS: &str = "mergeabilityChecks { identifier status }";

/// The fields [`parse_mr`] reads, shared by the list and a page's refresh.
macro_rules! mr_fields {
    () => {
        r#"
        iid title draft webUrl updatedAt sourceBranch targetBranch diffHeadSha userNotesCount
        conflicts detailedMergeStatus approved approvalsRequired approvalsLeft
        shouldBeRebased mergeabilityChecks { identifier status }
        author { username name }
        reviewers { nodes { username } }
        approvedBy { nodes { username } }
        headPipeline { status path duration stages { nodes { name status } } }
        resolvableDiscussionsCount resolvedDiscussionsCount
        notes(filter: ONLY_COMMENTS, last: 1) { nodes { url body createdAt author { username name } } }
        diffStatsSummary { additions deletions fileCount }
        project { name fullPath }
        state mergedAt closedAt
        labels { nodes { title } }
"#
    };
}

/// Merge requests of a group (and its subgroups) or of one project, newest
/// change first, `page` at a time. Any of state, `updatedAfter` and `iids`
/// may be null.
fn mrs_query(repo: bool, page: usize) -> String {
    let (container, scope) = if repo {
        ("project", "")
    } else {
        ("group", "includeSubgroups: true, ")
    };
    format!(
        r#"
query($path: ID!, $state: MergeRequestState, $after: String, $updatedAfter: Time, $iids: [String!]) {{
  currentUser {{ username }}
  {container}(fullPath: $path) {{
    mergeRequests({scope}state: $state, updatedAfter: $updatedAfter, iids: $iids, sort: UPDATED_DESC, first: {page}, after: $after) {{
      pageInfo {{ hasNextPage endCursor }}
      nodes {{ {fields} }}
    }}
  }}
}}"#,
        fields = mr_fields!()
    )
}

/// Everything a merge request's page shows: the list's fields (so the page's
/// refresh updates them too), the description, files, commits, the head
/// pipeline's jobs and every discussion.
const MR_DETAIL_QUERY: &str = concat!(
    r#"
query($project: ID!, $iid: String!) {
  project(fullPath: $project) {
    mergeRequest(iid: $iid) {"#,
    mr_fields!(),
    r#"
      description
      diffStats { path additions deletions }
      diffRefs { baseSha headSha startSha }
      squashOnMerge shouldRemoveSourceBranch forceRemoveSourceBranch
      commits(first: 100) { nodes { shortId title authorName authoredDate webUrl } }
      jobs: headPipeline {
        iid
        stages { nodes { name status jobs(first: 100) { nodes { name status duration webPath allowFailure } } } }
      }
      discussions(first: 100) {
        nodes {
          id resolved resolvable
          notes(first: 100) {
            nodes { body system createdAt author { username name } position { filePath newLine oldLine } }
          }
        }
      }
    }
  }
}"#
);

/// The latest pipelines on one branch of a project.
const RUNS_QUERY: &str = r#"
query($project: ID!, $ref: String) {
  project(fullPath: $project) {
    pipelines(ref: $ref, first: 10) {
      nodes {
        iid status createdAt duration path source
        user { name username }
        commit { shortId title }
      }
    }
  }
}"#;

/// What radar shows of a project: its default branch, that branch's newest
/// commit and latest pipeline.
macro_rules! project_fields {
    () => {
        r#"
        name fullPath archived lastActivityAt
        repository { rootRef tree { lastCommit { sha shortId authorName authoredDate title } } }
        pipelines(first: 10) { nodes { id sha ref status path updatedAt } }
"#
    };
}

// 50 a page: each project's newest commit costs GitLab a lookup.
const PROJECTS_QUERY: &str = concat!(
    r#"
query($path: ID!, $after: String) {
  group(fullPath: $path) {
    projects(includeSubgroups: true, first: 50, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes {"#,
    project_fields!(),
    r#"
      }
    }
  }
}"#
);

/// Each project's last activity only, 100 a page: what tells a projects
/// check which ones to fetch in full. Measured on gitlab.com for a group of
/// 21: score 40, 3 KB, 0.6 s, against 71, 25 KB, 3.2 s for [`PROJECTS_QUERY`].
const ACTIVITY_QUERY: &str = r#"
query($path: ID!, $after: String) {
  group(fullPath: $path) {
    projects(includeSubgroups: true, first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes { fullPath archived lastActivityAt }
    }
  }
}"#;

/// A single repo's last activity.
const REPO_ACTIVITY_QUERY: &str = r#"
query($path: ID!) {
  project(fullPath: $path) { fullPath archived lastActivityAt }
}"#;

/// `n` merge requests, each by its project (`$p0`…) and iid (`$i0`…),
/// answered as `m0`…, with only [`RECHECK_FIELDS`].
fn recheck_query(n: usize) -> String {
    let vars: Vec<String> = (0..n).map(|i| format!("$p{i}: ID!, $i{i}: String!")).collect();
    let asks: String = (0..n)
        .map(|i| {
            format!(
                "  m{i}: project(fullPath: $p{i}) {{ mergeRequest(iid: $i{i}) {{ {RECHECK_FIELDS} }} }}\n"
            )
        })
        .collect();
    format!("query({}) {{\n{asks}}}", vars.join(", "))
}

/// `n` projects in full, each by its path (`$p0`…), answered as `p0`….
fn projects_query(n: usize) -> String {
    let vars: Vec<String> = (0..n).map(|i| format!("$p{i}: ID!")).collect();
    let asks: String = (0..n)
        .map(|i| format!("  p{i}: project(fullPath: $p{i}) {{ {} }}\n", project_fields!()))
        .collect();
    format!("query({}) {{\n{asks}}}", vars.join(", "))
}

/// `n` projects' references only (branch, head commit, newest pipeline of
/// the branch asked as `$r0`…, which is the held default branch), each by
/// its path (`$p0`…), answered as `p0`…: what tells whether main moved, about
/// a seventh of the bytes of [`projects_query`]. The head commit cannot be asked
/// for every project of a group at once with the activity: 2.5 s against
/// 0.7 s for 21 projects on gitlab.com, for the one lookup per project.
fn pins_query(n: usize) -> String {
    let vars: Vec<String> = (0..n).map(|i| format!("$p{i}: ID!, $r{i}: String!")).collect();
    let asks: String = (0..n)
        .map(|i| {
            format!(
                "  p{i}: project(fullPath: $p{i}) {{ repository {{ rootRef tree {{ lastCommit {{ sha }} }} }} \
                 pipelines(ref: $r{i}, first: 1) {{ nodes {{ id status updatedAt }} }} }}\n"
            )
        })
        .collect();
    format!("query({}) {{\n{asks}}}", vars.join(", "))
}

const PROJECT_QUERY: &str = concat!(
    r#"
query($path: ID!) {
  project(fullPath: $path) {"#,
    project_fields!(),
    r#"
  }
}"#
);

/// Whether a path is a group or a project, and what is in it.
const PATH_QUERY: &str = r#"
query($path: ID!) {
  group(fullPath: $path) {
    fullName
    projects(includeSubgroups: true) { count }
    mergeRequests(state: opened, includeSubgroups: true) { count }
  }
  project(fullPath: $path) {
    nameWithNamespace
    mergeRequests(state: opened) { count }
  }
}"#;

impl Client {
    pub fn new(url: &str, token: String) -> Self {
        let base = url.trim_end_matches('/').to_string();
        let http = Http {
            http: shared_http(&base).expect("http client"),
            base: base.clone(),
            token,
        };
        Self {
            t: Arc::new(http),
            base,
            pacing: Arc::default(),
        }
    }

    /// A client over another transport (recorded replies).
    #[cfg(any(test, feature = "test-support"))]
    pub fn over(t: Arc<dyn Transport>) -> Self {
        Self::over_at(t, "https://gitlab.example.com")
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn over_at(t: Arc<dyn Transport>, server: &str) -> Self {
        Self { t, base: server.trim_end_matches('/').into(), pacing: Arc::default() }
    }

    /// What the latest answer said about how fast GitLab wants to be asked.
    pub fn pacing(&self) -> Pacing {
        *self.pacing.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One request. No answer at all is a [`Trouble`], which the caller
    /// waits out; every status is left to the caller (a 5xx may still carry
    /// usable data).
    async fn send(&self, req: Req) -> Result<Resp> {
        let resp = self
            .t
            .send(&req)
            .await
            .map_err(|e| anyhow::Error::from(Trouble::unreachable(e)))?;
        *self.pacing.lock().unwrap_or_else(|e| e.into_inner()) = resp.pacing;
        Ok(resp)
    }

    async fn query(&self, query: &str, variables: Value) -> Result<Value> {
        match self.query_once(query, variables.clone()).await {
            // A server older than mergeability checks: ask without them; the
            // state then comes from the merge request's own fields.
            Err(e) if query.contains(CHECKS) && e.to_string().contains("mergeabilityChecks") => {
                self.query_once(&query.replace(CHECKS, ""), variables).await
            }
            other => other,
        }
    }

    async fn query_once(&self, query: &str, variables: Value) -> Result<Value> {
        let resp = self
            .send(Req::new(
                Method::Post,
                "/api/graphql",
                Some(json!({ "query": query, "variables": variables })),
            ))
            .await?;
        let status = resp.status;
        if status == 401 {
            return Err(Rejected.into());
        }
        // An outage or a proxy answers with an HTML page: say the status, not
        // the parser's complaint, and back off when it is one to wait out.
        let body: Value = match serde_json::from_slice(&resp.body) {
            Ok(body) => body,
            Err(_) if backs_off(status) => {
                return Err(answered(&resp, &Value::Null));
            }
            Err(_) if !(200..300).contains(&status) => {
                bail!("GitLab answered {}", status_words(status));
            }
            Err(e) => return Err(e).with_context(|| format!("GitLab answered {status}")),
        };
        if let Some(errors) = body
            .get("errors")
            .and_then(Value::as_array)
            .filter(|e| !e.is_empty())
        {
            let msg = errors
                .iter()
                .filter_map(|e| e["message"].as_str())
                .collect::<Vec<_>>()
                .join("; ");
            if backs_off(status) {
                return Err(Trouble::answered(status, format!("GitLab: {msg}"), resp.pacing).into());
            }
            bail!("GitLab: {msg}");
        }
        Ok(body["data"].clone())
    }

    /// Who this token belongs to: a sentence for the user.
    pub async fn check_user(&self) -> Result<String> {
        self.check_user_observed().await.map(|got| got.value)
    }

    /// Whether a path is a group or a repo and is visible, with how much is
    /// in it: a sentence for the user.
    pub async fn check_path(&self, full_path: &str) -> Result<String> {
        self.check_path_observed(full_path)
            .await
            .map(|got| got.value)
    }

    /// `true` when the path is a repo, `false` for a group.
    async fn is_repo(&self, full_path: &str) -> Result<bool> {
        let d = self.query(PATH_QUERY, json!({ "path": full_path })).await?;
        if !d["group"].is_null() {
            Ok(false)
        } else if !d["project"].is_null() {
            Ok(true)
        } else {
            bail!("no group or repo {full_path}, or this token cannot see it")
        }
    }

    /// Up to `pages` pages of merge requests; `state` `None` is any state.
    #[allow(clippy::too_many_arguments)]
    async fn mrs(
        &self,
        query: &str,
        path: &str,
        state: Option<&str>,
        updated_after: Option<&str>,
        iids: Option<Vec<String>>,
        pages: usize,
        me: &mut String,
    ) -> Result<MrBatch> {
        let base = self.base();
        let mut out = MrBatch::default();
        let mut after = Value::Null;
        for _ in 0..pages {
            let d = self
                .query(
                    query,
                    json!({ "path": path, "state": state, "after": after,
                            "updatedAfter": updated_after, "iids": iids }),
                )
                .await?;
            if me.is_empty() {
                *me = str_of(&d["currentUser"]["username"]);
            }
            let container = if d["group"].is_object() {
                &d["group"]
            } else {
                &d["project"]
            };
            let conn = &container["mergeRequests"];
            if conn.is_null() {
                bail!("{path} not found, or this token cannot see it");
            }
            out.mrs.extend(nodes(conn).map(|n| parse_mr(n, &base)));
            match next_page(conn) {
                Some(cursor) => {
                    after = cursor;
                    out.truncated = true;
                }
                None => {
                    out.truncated = false;
                    break;
                }
            }
        }
        Ok(out)
    }

    /// One sync of a tenancy, as [`Want`] says.
    pub async fn sync(&self, path: &str, want: &Want) -> Result<Synced> {
        let repo = match want.repo {
            Some(r) => r,
            None => self.is_repo(path).await?,
        };
        let q = mrs_query(repo, PAGE);
        let mut out = Synced {
            repo,
            ..Default::default()
        };
        match &want.changed_since {
            None => {
                let opened = self
                    .mrs(&q, path, Some("opened"), None, None, usize::MAX, &mut out.me)
                    .await?;
                out.mrs = opened.mrs;
                out.mrs_truncated = opened.truncated;
                if let Some(since) = &want.ended_since {
                    for state in ["merged", "closed"] {
                        let ended = self
                            .mrs(&q, path, Some(state), Some(since), None, MAX_PAGES, &mut out.me)
                            .await?;
                        out.mrs_truncated |= ended.truncated;
                        out.mrs.extend(ended.mrs);
                    }
                }
            }
            Some(since) => {
                // Ask GitLab to check the ones it has not; what it found
                // comes with this sync's recheck or the next.
                out.nudges.deferred = want.nudge.len().saturating_sub(MOST_NUDGES);
                for (project, iid) in want.nudge.iter().take(MOST_NUDGES) {
                    out.nudges.attempted += 1;
                    if self
                        .get_rest(&format!(
                            "projects/{}/merge_requests/{iid}?with_merge_status_recheck=true",
                            encoded(project)
                        ))
                        .await
                        .is_err()
                    {
                        out.nudges.errors += 1;
                    }
                }
                let changed = self
                    .mrs(&q, path, None, Some(since), None, MAX_PAGES, &mut out.me)
                    .await?;
                out.mrs = changed.mrs;
                out.mrs_truncated = changed.truncated;
                let mut again: Vec<&(String, u64)> = want
                    .recheck
                    .iter()
                    .filter(|(p, i)| !out.mrs.iter().any(|m| m.project_path == *p && m.iid == *i))
                    .collect();
                again.sort();
                again.dedup();
                out.rechecked = self.recheck(&again).await?;
            }
        }
        if want.projects {
            match &want.projects_check {
                None => {
                    out.projects = Some(if repo {
                        self.project(path).await?.into_iter().collect()
                    } else {
                        self.projects(path).await?
                    });
                }
                Some(check) => {
                    let present = self.activity(path, repo).await?;
                    // A merge lands a commit on the target branch.
                    let merged: BTreeSet<&str> = out
                        .mrs
                        .iter()
                        .filter(|m| m.state == MrState::Merged)
                        .map(|m| m.project_path.as_str())
                        .collect();
                    // In full: new ones, ones main's pipeline runs in, ones a
                    // merge landed in, and moved or recent ones with no
                    // reference. Moved or recent ones with one are looked at
                    // by it first.
                    let mut stale: Vec<String> = Vec::new();
                    let mut look: Vec<String> = Vec::new();
                    for (p, at) in &present {
                        let must = check.again.contains(p)
                            || merged.contains(p.as_str())
                            || !check.seen.contains_key(p);
                        let moved = check.seen.get(p) != Some(at) || check.look.contains(p);
                        if must || (moved && !check.pins.contains_key(p)) {
                            stale.push(p.clone());
                        } else if moved {
                            look.push(p.clone());
                        }
                    }
                    for (p, now) in self.pins(&look, &check.pins).await? {
                        if now.as_ref() != check.pins.get(&p) {
                            stale.push(p);
                        }
                    }
                    stale.sort();
                    let fresh = self.projects_by_path(&stale).await?;
                    out.projects_seen = Some(ProjectsSeen { present, fresh });
                }
            }
        }
        Ok(out)
    }

    /// Open merge requests again, each asked of its own project
    /// ([`recheck_query`]), [`RECHECK_BATCH`] a query. One GitLab no longer
    /// shows is left out.
    async fn recheck(&self, again: &[&(String, u64)]) -> Result<Vec<Mr>> {
        let base = self.base();
        let mut out = Vec::new();
        for chunk in again.chunks(RECHECK_BATCH) {
            let mut vars = serde_json::Map::new();
            for (i, (project, iid)) in chunk.iter().enumerate() {
                vars.insert(format!("p{i}"), json!(project));
                vars.insert(format!("i{i}"), json!(iid.to_string()));
            }
            let d = self
                .query(&recheck_query(chunk.len()), Value::Object(vars))
                .await?;
            for (i, (project, iid)) in chunk.iter().enumerate() {
                let n = &d[format!("m{i}")]["mergeRequest"];
                if n.is_object() {
                    let mut mr = parse_mr(n, &base);
                    mr.project_path = project.clone();
                    mr.iid = *iid;
                    out.push(mr);
                }
            }
        }
        Ok(out)
    }

    /// Every unarchived project's path and last activity, by path.
    async fn activity(&self, path: &str, repo: bool) -> Result<Vec<(String, String)>> {
        let row = |n: &Value| {
            (n["archived"].as_bool() != Some(true) && n["fullPath"].is_string())
                .then(|| (str_of(&n["fullPath"]), str_of(&n["lastActivityAt"])))
        };
        let mut out = Vec::new();
        if repo {
            let d = self.query(REPO_ACTIVITY_QUERY, json!({ "path": path })).await?;
            if d["project"].is_null() {
                bail!("no repo {path}, or this token cannot see it");
            }
            out.extend(row(&d["project"]));
        } else {
            let mut after = Value::Null;
            loop {
                let d = self
                    .query(ACTIVITY_QUERY, json!({ "path": path, "after": after }))
                    .await?;
                let conn = &d["group"]["projects"];
                out.extend(nodes(conn).filter_map(row));
                match next_page(conn) {
                    Some(cursor) => after = cursor,
                    None => break,
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Projects' references now ([`pins_query`]), [`PINS_BATCH`] a query,
    /// each asked of its branch as held in `held`; `None` for one GitLab
    /// gives no answer for (gone, or not visible).
    async fn pins(
        &self,
        paths: &[String],
        held: &BTreeMap<String, MainRef>,
    ) -> Result<Vec<(String, Option<MainRef>)>> {
        let mut out = Vec::new();
        for chunk in paths.chunks(PINS_BATCH) {
            let mut vars = serde_json::Map::new();
            for (i, p) in chunk.iter().enumerate() {
                vars.insert(format!("p{i}"), json!(p));
                vars.insert(format!("r{i}"), json!(held.get(p).map_or("", |h| h.branch.as_str())));
            }
            let d = self.query(&pins_query(chunk.len()), Value::Object(vars)).await?;
            out.extend(
                chunk
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.clone(), parse_pin(&d[format!("p{i}")]))),
            );
        }
        Ok(out)
    }

    /// Projects in full, each by its path ([`projects_query`]),
    /// [`PROJECTS_BATCH`] a query.
    async fn projects_by_path(&self, paths: &[String]) -> Result<Vec<Project>> {
        let base = self.base();
        let mut out = Vec::new();
        for chunk in paths.chunks(PROJECTS_BATCH) {
            let vars: serde_json::Map<String, Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, p)| (format!("p{i}"), json!(p)))
                .collect();
            let d = self
                .query(&projects_query(chunk.len()), Value::Object(vars))
                .await?;
            out.extend((0..chunk.len()).filter_map(|i| {
                let n = &d[format!("p{i}")];
                n.is_object().then(|| parse_project(n, &base)).flatten()
            }));
        }
        Ok(out)
    }

    /// Every unarchived project in a group and its subgroups, by path.
    async fn projects(&self, group: &str) -> Result<Vec<Project>> {
        let base = self.base();
        let mut out = Vec::new();
        let mut after = Value::Null;
        loop {
            let d = self
                .query(PROJECTS_QUERY, json!({ "path": group, "after": after }))
                .await?;
            let conn = &d["group"]["projects"];
            out.extend(nodes(conn).filter_map(|n| parse_project(n, &base)));
            match next_page(conn) {
                Some(cursor) => after = cursor,
                None => break,
            }
        }
        out.sort_by(|a, b| a.full_path.cmp(&b.full_path));
        Ok(out)
    }

    async fn project(&self, path: &str) -> Result<Option<Project>> {
        let d = self.query(PROJECT_QUERY, json!({ "path": path })).await?;
        if d["project"].is_null() {
            bail!("no repo {path}, or this token cannot see it");
        }
        Ok(parse_project(&d["project"], &self.base()))
    }

    fn base(&self) -> String {
        self.base.clone()
    }

    /// One merge request as the list has it, and what its page shows.
    pub async fn mr_detail(&self, project: &str, iid: u64) -> Result<(Mr, MrDetail)> {
        let got = self.mr_detail_observed(project, iid).await?;
        Ok((got.mr, got.detail))
    }

    /// The same request and parsers, with transient structural evidence.
    pub(crate) async fn mr_detail_observed(&self, project: &str, iid: u64) -> Result<MrDetailRead> {
        let d = self
            .query(
                MR_DETAIL_QUERY,
                json!({ "project": project, "iid": iid.to_string() }),
            )
            .await?;
        let n = &d["project"]["mergeRequest"];
        if n.is_null() {
            bail!("no merge request !{iid} in {project}, or this token cannot see it");
        }
        let base = self.base();
        Ok(MrDetailRead {
            mr: parse_mr(n, &base),
            detail: parse_detail(n, &base),
            evidence: detail_evidence(n),
            raw_iid: n["iid"]
                .as_u64()
                .or_else(|| n["iid"].as_str().and_then(|s| s.parse().ok())),
        })
    }

    /// The latest pipelines on `branch`, newest first.
    pub async fn runs(&self, project: &str, branch: &str) -> Result<Vec<PipelineRun>> {
        Ok(self.runs_observed(project, branch).await?.runs)
    }

    /// The same bounded query and parser, with transient structural evidence.
    pub(crate) async fn runs_observed(&self, project: &str, branch: &str) -> Result<RunsRead> {
        let d = self
            .query(RUNS_QUERY, json!({ "project": project, "ref": branch }))
            .await?;
        if d["project"].is_null() {
            bail!("no repo {project}, or this token cannot see it");
        }
        let conn = &d["project"]["pipelines"];
        let rows = conn["nodes"].as_array();
        Ok(RunsRead {
            runs: parse_runs(conn, &self.base()),
            evidence: RunsEvidence {
                incomplete_response: rows.is_none(),
                incomplete_rows: rows.map_or(0, |rows| {
                    rows.iter().filter(|row| !row.is_object()).count()
                }),
            },
        })
    }
}

/// Requests that change something on GitLab: built and read in
/// `mr_actions`, sent here.
impl Client {
    /// A GraphQL query or mutation of the caller's own.
    pub async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        self.query(query, variables).await
    }

    /// `GET /api/v4/<path>`: its JSON answer, or what GitLab said.
    async fn get_rest(&self, path: &str) -> Result<Value> {
        let resp = self
            .send(Req::new(Method::Get, format!("/api/v4/{path}"), None))
            .await?;
        if resp.status == 401 {
            return Err(Rejected.into());
        }
        let body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        if !(200..300).contains(&resp.status) {
            return Err(answered(&resp, &body));
        }
        Ok(body)
    }

    /// A pipeline's failed jobs (the latest try of each), in GitLab's order.
    pub async fn failed_jobs(&self, project: &str, pipeline: u64) -> Result<Vec<JobInfo>> {
        Ok(self.failed_jobs_observed(project, pipeline).await?.jobs)
    }

    /// The same request and parser, with transient list-shape evidence only.
    pub(crate) async fn failed_jobs_observed(
        &self,
        project: &str,
        pipeline: u64,
    ) -> Result<FailedJobsRead> {
        let v = self
            .get_rest(&format!(
                "projects/{}/pipelines/{pipeline}/jobs?scope[]=failed&per_page=100",
                encoded(project)
            ))
            .await?;
        let rows = v.as_array();
        Ok(FailedJobsRead {
            jobs: rows
                .into_iter()
                .flatten()
                .map(|j| parse_job(j, project))
                .collect(),
            evidence: FailedJobsEvidence {
                incomplete_response: rows.is_none(),
                incomplete_rows: rows
                    .into_iter()
                    .flatten()
                    .filter(|row| !row.is_object())
                    .count(),
            },
        })
    }

    /// One job.
    pub async fn job(&self, project: &str, id: u64) -> Result<JobInfo> {
        Ok(self.job_observed(project, id).await?.info)
    }

    /// The same permissive parser, with response-shape evidence only.
    pub(crate) async fn job_observed(&self, project: &str, id: u64) -> Result<JobRead> {
        let v = self
            .get_rest(&format!("projects/{}/jobs/{id}", encoded(project)))
            .await?;
        Ok(JobRead {
            info: parse_job(&v, project),
            evidence: JobEvidence {
                nonobject_response: !v.is_object(),
            },
        })
    }

    /// A part of a job's log: the last [`TRACE_PART`] bytes, or the part
    /// before byte `before`. GitLab sends the whole log when it does not
    /// take a range (a job still running).
    pub async fn trace(&self, project: &str, id: u64, before: Option<u64>) -> Result<TracePart> {
        Ok(self.trace_observed(project, id, before).await?.part)
    }

    pub(crate) async fn trace_observed(
        &self,
        project: &str,
        id: u64,
        before: Option<u64>,
    ) -> Result<TraceRead> {
        let mut req = Req::new(
            Method::Get,
            format!("/api/v4/projects/{}/jobs/{id}/trace", encoded(project)),
            None,
        );
        req.range = Some(trace_range(before));
        let resp = self.send(req).await?;
        if resp.status == 401 {
            return Err(Rejected.into());
        }
        // Legacy default: neither this status nor ignored bytes prove an empty log.
        if resp.status == 416 {
            return Ok(TraceRead {
                part: TracePart::default(),
                evidence: TraceEvidence {
                    supplied_bytes: resp.body.len(),
                    interpretation: TraceInterpretation::Legacy416Default,
                },
            });
        }
        if !(200..300).contains(&resp.status) {
            let body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
            return Err(answered(&resp, &body));
        }
        Ok(trace_part_observed(
            &resp.body,
            resp.content_range.as_deref(),
        ))
    }

    /// `PUT /api/v4/<path>`, sent once: GitLab's status code and its JSON answer.
    pub async fn put_rest(&self, path: &str) -> Result<(u16, Value)> {
        self.write(Req::new(Method::Put, format!("/api/v4/{path}"), None))
            .await
    }

    /// A request that changes something, sent once and never again
    /// whatever comes back: GitLab's status code and its JSON answer.
    pub async fn write(&self, req: Req) -> Result<(u16, Value)> {
        radar_core::log::info(format!(
            "gitlab write: {:?} {}",
            req.method,
            req.path.split('?').next().unwrap_or("")
        ));
        let resp = self.send(req).await?;
        Ok((
            resp.status,
            serde_json::from_slice(&resp.body).unwrap_or(Value::Null),
        ))
    }
}

/// `404 Not Found`: a status as GitLab's server says it.
/// A status GitLab sends when it is down or limiting us: it is waited out.
fn backs_off(status: u16) -> bool {
    status == 429 || status >= 500
}

/// What GitLab's refusal of a REST request says. A [`Trouble`] when it is
/// to be waited out, so the caller can back off.
fn answered(resp: &Resp, body: &Value) -> anyhow::Error {
    let message = format!(
        "GitLab answered {}{}",
        status_words(resp.status),
        rest_message(body)
    );
    if backs_off(resp.status) {
        Trouble::answered(resp.status, message, resp.pacing).into()
    } else {
        anyhow::anyhow!(message)
    }
}

fn status_words(status: u16) -> String {
    match reqwest::StatusCode::from_u16(status) {
        Ok(s) => s.to_string(),
        Err(_) => status.to_string(),
    }
}

/// A project's path as one segment of a REST path.
fn encoded(project: &str) -> String {
    project.replace('/', "%2F")
}

/// ` · <message>` from a REST error answer, when it has one.
fn rest_message(body: &Value) -> String {
    let m = body["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or_default();
    if m.is_empty() {
        String::new()
    } else {
        format!(" · {m}")
    }
}

/// The `Range` header for [`Client::trace`].
pub fn trace_range(before: Option<u64>) -> String {
    match before {
        None => format!("bytes=-{TRACE_PART}"),
        Some(end) => format!("bytes={}-{}", end.saturating_sub(TRACE_PART), end.saturating_sub(1)),
    }
}

/// A log's bytes and its `Content-Range` (`bytes 4446-6445/6446`), or the
/// whole log when there is none.
#[allow(dead_code)] // Retain the legacy pure-parser entry point beside its observed wrapper.
pub fn trace_part(bytes: &[u8], content_range: Option<&str>) -> TracePart {
    trace_part_observed(bytes, content_range).part
}

fn trace_part_observed(bytes: &[u8], content_range: Option<&str>) -> TraceRead {
    let text = String::from_utf8_lossy(bytes);
    let lossy_utf8 = matches!(text, std::borrow::Cow::Owned(_));
    let text = text.into_owned();
    let len = bytes.len() as u64;
    let parsed = content_range.and_then(|r| {
        let (span, total) = r.strip_prefix("bytes ")?.split_once('/')?;
        let start = span.split_once('-')?.0.parse().ok()?;
        Some((start, total.parse().ok()?))
    });
    let range = match parsed {
        Some((start, total)) => TraceRange::Parsed { start, total },
        None if content_range.is_none() => TraceRange::Missing,
        None => TraceRange::Unparseable,
    };
    let (start, total) = parsed.unwrap_or((0, len));
    TraceRead {
        part: TracePart { text, start, total },
        evidence: TraceEvidence {
            supplied_bytes: bytes.len(),
            interpretation: TraceInterpretation::AcceptedBody { range, lossy_utf8 },
        },
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JobEvidence {
    pub nonobject_response: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct JobRead {
    pub info: JobInfo,
    pub evidence: JobEvidence,
}

/// Transient facts from the existing parser, never range-validity evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TraceRange {
    Parsed { start: u64, total: u64 },
    Missing,
    Unparseable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TraceInterpretation {
    AcceptedBody { range: TraceRange, lossy_utf8: bool },
    Legacy416Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TraceEvidence {
    /// Bytes supplied by Resp, not proof HTTP's body read completed.
    pub supplied_bytes: usize,
    pub interpretation: TraceInterpretation,
}

#[derive(Debug, Clone)]
pub(crate) struct TraceRead {
    pub part: TracePart,
    pub evidence: TraceEvidence,
}

/// `…/-/jobs/123` → 123.
pub fn job_id_of(path: &str) -> Option<u64> {
    number_after(path, "/-/jobs/")
}

/// `…/-/pipelines/123` → 123.
pub fn pipeline_id_of(path: &str) -> Option<u64> {
    number_after(path, "/-/pipelines/")
}

fn number_after(path: &str, marker: &str) -> Option<u64> {
    let (_, rest) = path.rsplit_once(marker)?;
    rest.split(['/', '?', '#']).next()?.parse().ok()
}

/// A job from GitLab's jobs API.
pub fn parse_job(j: &Value, project: &str) -> JobInfo {
    JobInfo {
        id: j["id"].as_u64().unwrap_or(0),
        project: project.to_string(),
        name: str_of(&j["name"]),
        stage: str_of(&j["stage"]),
        status: str_of(&j["status"]).to_ascii_lowercase(),
        duration_secs: j["duration"].as_f64().map(|d| d.round() as u32),
        failure_reason: str_of(&j["failure_reason"]),
        url: str_of(&j["web_url"]),
        allow_failure: j["allow_failure"].as_bool().unwrap_or(false),
        pipeline: j["pipeline"]["id"].as_u64().unwrap_or(0),
    }
}

/// Structural evidence only, never serialized into the provider cache.
/// Scalar and nested job fields retain the existing permissive parser behavior.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FailedJobsEvidence {
    pub incomplete_response: bool,
    pub incomplete_rows: usize,
}

#[derive(Debug)]
pub(crate) struct FailedJobsRead {
    pub jobs: Vec<JobInfo>,
    pub evidence: FailedJobsEvidence,
}

fn nodes(v: &Value) -> impl Iterator<Item = &Value> {
    v["nodes"].as_array().into_iter().flatten()
}

/// A GraphQL global id's number: `gid://gitlab/Ci::Pipeline/498` is 498.
fn gid_number(v: &Value) -> Option<u64> {
    v.as_str()?.rsplit('/').next()?.parse().ok()
}

/// A project's reference from a pin query's answer ([`pins_query`]); `None`
/// when GitLab gave none (the project is gone or not visible).
fn parse_pin(n: &Value) -> Option<MainRef> {
    if !n.is_object() {
        return None;
    }
    let pipeline = nodes(&n["pipelines"]).next();
    Some(MainRef {
        branch: str_of(&n["repository"]["rootRef"]),
        head_sha: str_of(&n["repository"]["tree"]["lastCommit"]["sha"]),
        pipeline_id: pipeline.and_then(|p| gid_number(&p["id"])),
        pipeline: pipeline
            .and_then(|p| p["status"].as_str())
            .and_then(Pipeline::from_graphql),
        pipeline_at: pipeline.map(|p| str_of(&p["updatedAt"])).unwrap_or_default(),
    })
}

/// A project node; `None` when it is archived.
pub fn parse_project(n: &Value, base: &str) -> Option<Project> {
    if n["archived"].as_bool() == Some(true) {
        return None;
    }
    let default_branch = str_of(&n["repository"]["rootRef"]);
    // The newest pipeline on the default branch among the recent ones.
    let main = nodes(&n["pipelines"]).find(|p| {
        !default_branch.is_empty() && p["ref"].as_str() == Some(default_branch.as_str())
    });
    let commit = &n["repository"]["tree"]["lastCommit"];
    Some(Project {
        name: str_of(&n["name"]),
        full_path: str_of(&n["fullPath"]),
        main_pipeline: main
            .and_then(|p| p["status"].as_str())
            .and_then(Pipeline::from_graphql),
        main_pipeline_url: main.and_then(|p| url_of(&p["path"], base)),
        main_pipeline_id: main.and_then(|p| gid_number(&p["id"])),
        main_pipeline_at: main.map(|p| str_of(&p["updatedAt"])).unwrap_or_default(),
        last_activity: str_of(&n["lastActivityAt"]),
        default_branch,
        main_commit: (!commit.is_null()).then(|| MainCommit {
            sha: str_of(&commit["sha"]),
            short_id: str_of(&commit["shortId"]),
            author: str_of(&commit["authorName"]),
            at: str_of(&commit["authoredDate"]),
            title: text_of(&commit["title"]),
        }),
    })
}

fn secs_of(v: &Value) -> Option<u32> {
    v.as_u64().map(|d| d as u32)
}

fn url_of(v: &Value, base: &str) -> Option<String> {
    v.as_str().map(|p| format!("{base}{p}"))
}

/// The page's part of a merge request node from [`MR_DETAIL_QUERY`].
pub fn parse_detail(n: &Value, base: &str) -> MrDetail {
    let pipeline = (!n["jobs"].is_null()).then(|| PipelineDetail {
        iid: str_of(&n["jobs"]["iid"]),
        stages: nodes(&n["jobs"]["stages"])
            .map(|s| StageJobs {
                name: str_of(&s["name"]),
                status: str_of(&s["status"]).to_ascii_lowercase(),
                jobs: nodes(&s["jobs"])
                    .map(|j| Job {
                        id: j["webPath"].as_str().and_then(job_id_of),
                        name: str_of(&j["name"]),
                        status: str_of(&j["status"]).to_ascii_lowercase(),
                        duration_secs: secs_of(&j["duration"]),
                        url: url_of(&j["webPath"], base),
                        allow_failure: j["allowFailure"].as_bool().unwrap_or(false),
                    })
                    .collect(),
            })
            .collect(),
    });
    let discussions = nodes(&n["discussions"])
        .map(|d| Discussion {
            id: str_of(&d["id"]),
            resolvable: d["resolvable"].as_bool().unwrap_or(false),
            resolved: d["resolved"].as_bool().unwrap_or(false),
            notes: nodes(&d["notes"])
                .filter(|note| note["system"].as_bool() != Some(true))
                .map(|note| {
                    let name = str_of(&note["author"]["name"]);
                    Note {
                        author: if name.is_empty() {
                            str_of(&note["author"]["username"])
                        } else {
                            name
                        },
                        body: str_of(&note["body"]),
                        at: str_of(&note["createdAt"]),
                        path: note["position"]["filePath"].as_str().map(String::from),
                        line: secs_of(&note["position"]["newLine"])
                            .or_else(|| secs_of(&note["position"]["oldLine"])),
                        old_line: secs_of(&note["position"]["oldLine"]),
                        new_line: secs_of(&note["position"]["newLine"]),
                    }
                })
                .collect::<Vec<_>>(),
        })
        .filter(|d| !d.notes.is_empty())
        .collect();
    MrDetail {
        description: str_of(&n["description"]),
        files: n["diffStats"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| FileStat {
                path: str_of(&f["path"]),
                additions: u32_of(&f["additions"]),
                deletions: u32_of(&f["deletions"]),
            })
            .collect(),
        discussions,
        pipeline,
        commits: nodes(&n["commits"])
            .map(|c| CommitInfo {
                short_id: str_of(&c["shortId"]),
                title: text_of(&c["title"]),
                author: str_of(&c["authorName"]),
                at: str_of(&c["authoredDate"]),
                url: str_of(&c["webUrl"]),
            })
            .collect(),
        refs: n["diffRefs"].is_object().then(|| DiffRefs {
            base: str_of(&n["diffRefs"]["baseSha"]),
            head: str_of(&n["diffRefs"]["headSha"]),
            start: str_of(&n["diffRefs"]["startSha"]),
        }),
        merge: MergeSettings {
            squash: n["squashOnMerge"].as_bool().unwrap_or(false),
            remove_source: n["shouldRemoveSourceBranch"].as_bool().unwrap_or(false)
                || n["forceRemoveSourceBranch"].as_bool().unwrap_or(false),
            remove_forced: n["forceRemoveSourceBranch"].as_bool().unwrap_or(false),
        },
    }
}

/// Structural evidence only; scalar defaults remain the existing parser's job.
/// A flag also covers an unusable enclosing shape, not an estimated missing row.
/// None of this is serialized into the provider cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MrDetailEvidence {
    pub mr_incomplete: bool,
    pub files_incomplete: bool,
    pub commits_incomplete: bool,
    pub discussions_incomplete: bool,
    pub notes_incomplete: bool,
    pub pipeline_incomplete: bool,
    pub stages_incomplete: bool,
    pub jobs_incomplete: bool,
}

#[derive(Debug)]
pub(crate) struct MrDetailRead {
    pub mr: Mr,
    pub detail: MrDetail,
    pub evidence: MrDetailEvidence,
    // Lossless before the legacy parser's u32 narrowing; never serialized or
    // used as journal scope. None also covers an unrepresentable raw value.
    raw_iid: Option<u64>,
}

impl MrDetailRead {
    pub(crate) fn matches_iid(&self, requested: u64) -> bool {
        requested > 0 && self.raw_iid == Some(requested) && self.mr.iid == requested
    }
}

fn detail_rows<'a>(value: &'a Value, incomplete: &mut bool) -> &'a [Value] {
    match value.as_array() {
        Some(rows) => rows,
        None => {
            *incomplete = true;
            &[]
        }
    }
}

fn malformed_optional_object(value: &Value) -> bool {
    !value.is_null() && !value.is_object()
}

fn detail_evidence(n: &Value) -> MrDetailEvidence {
    let mut e = MrDetailEvidence::default();
    if !n.is_object() {
        // One malformed parent, not eight independently observed gaps.
        return MrDetailEvidence {
            mr_incomplete: true,
            files_incomplete: true,
            commits_incomplete: true,
            discussions_incomplete: true,
            notes_incomplete: true,
            pipeline_incomplete: true,
            stages_incomplete: true,
            jobs_incomplete: true,
        };
    }
    e.mr_incomplete = malformed_optional_object(&n["diffRefs"]);
    for row in detail_rows(&n["diffStats"], &mut e.files_incomplete) {
        e.files_incomplete |= !row.is_object();
    }
    for row in detail_rows(&n["commits"]["nodes"], &mut e.commits_incomplete) {
        e.commits_incomplete |= !row.is_object();
    }
    for discussion in detail_rows(&n["discussions"]["nodes"], &mut e.discussions_incomplete) {
        if !discussion.is_object() {
            e.discussions_incomplete = true;
            continue;
        }
        for note in detail_rows(&discussion["notes"]["nodes"], &mut e.notes_incomplete) {
            // These notes are deliberately excluded, even if their unused content
            // is malformed. Empty and system-only discussions are normal too.
            if note["system"].as_bool() == Some(true) {
                continue;
            }
            e.notes_incomplete |= !note.is_object()
                || malformed_optional_object(&note["author"])
                || malformed_optional_object(&note["position"]);
        }
    }
    e.notes_incomplete |= e.discussions_incomplete;
    let pipeline = &n["jobs"];
    if malformed_optional_object(pipeline) {
        e.pipeline_incomplete = true;
        e.stages_incomplete = true;
        e.jobs_incomplete = true;
    } else if pipeline.is_object() {
        for stage in detail_rows(&pipeline["stages"]["nodes"], &mut e.stages_incomplete) {
            if !stage.is_object() {
                e.stages_incomplete = true;
                continue;
            }
            for job in detail_rows(&stage["jobs"]["nodes"], &mut e.jobs_incomplete) {
                e.jobs_incomplete |= !job.is_object();
            }
        }
        e.jobs_incomplete |= e.stages_incomplete;
    }
    e
}

/// One file of GitLab's diffs API.
pub fn parse_file_diff(f: &Value) -> FileDiff {
    FileDiff {
        old_path: str_of(&f["old_path"]),
        new_path: str_of(&f["new_path"]),
        new_file: f["new_file"].as_bool().unwrap_or(false),
        renamed: f["renamed_file"].as_bool().unwrap_or(false),
        deleted: f["deleted_file"].as_bool().unwrap_or(false),
        diff: str_of(&f["diff"]),
        too_large: f["too_large"].as_bool().unwrap_or(false)
            || f["collapsed"].as_bool().unwrap_or(false),
    }
}

/// How many files one diffs request asks for. GitLab's REST pagination
/// allows up to 100 and defaults to 20; a 100-file merge request is five
/// requests, not twenty.
pub const DIFF_PAGE: usize = 20;

/// Evidence about one returned diff page, never persisted in the provider cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffEvidence {
    /// The legacy parser received no array (including invalid JSON).
    pub incomplete_response: bool,
    /// Entries with missing/malformed fields that the legacy parser defaults.
    /// Count each entry once; the entries themselves are still returned.
    pub incomplete_rows: usize,
    /// Entries explicitly marked collapsed or too large, counted once each.
    pub withheld_rows: usize,
}

#[derive(Debug, Default)]
pub(crate) struct DiffRead {
    pub files: Vec<FileDiff>,
    pub evidence: DiffEvidence,
}

impl Client {
    /// One page of a merge request's file diffs ([`DIFF_PAGE`] files), in
    /// GitLab's order.
    pub async fn diffs(&self, project: &str, iid: u64, page: usize) -> Result<Vec<FileDiff>> {
        Ok(self.diffs_observed(project, iid, page).await?.files)
    }

    /// The same single-page request and permissive parser, with count-only gaps.
    /// This endpoint is not pinned to a revision and exposes no pagination
    /// continuation here. A full page does not establish a cutoff.
    pub(crate) async fn diffs_observed(
        &self,
        project: &str,
        iid: u64,
        page: usize,
    ) -> Result<DiffRead> {
        let v = self
            .get_rest(&format!(
                "projects/{}/merge_requests/{iid}/diffs?page={page}&per_page={DIFF_PAGE}",
                encoded(project)
            ))
            .await?;
        let rows = v.as_array();
        let mut evidence = DiffEvidence {
            incomplete_response: rows.is_none(),
            ..Default::default()
        };
        let files = rows
            .into_iter()
            .flatten()
            .map(|row| {
                let withheld = row["too_large"].as_bool() == Some(true)
                    || row["collapsed"].as_bool() == Some(true);
                evidence.withheld_rows += usize::from(withheld);
                let missing_path = ["old_path", "new_path"]
                    .iter()
                    .any(|key| row[*key].as_str().is_none_or(str::is_empty));
                let malformed_flags = [
                    "new_file",
                    "renamed_file",
                    "deleted_file",
                    "too_large",
                    "collapsed",
                ]
                .iter()
                .any(|key| row.get(*key).is_some_and(|v| !v.is_boolean()));
                evidence.incomplete_rows += usize::from(
                    missing_path || malformed_flags || (!withheld && !row["diff"].is_string()),
                );
                parse_file_diff(row)
            })
            .collect();
        Ok(DiffRead { files, evidence })
    }
}

/// Structural facts from one returned connection, never cached or serialized.
/// Scalar defaults remain the legacy parser's job; counts are row occurrences.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RunsEvidence {
    pub incomplete_response: bool,
    pub incomplete_rows: usize,
}

#[derive(Debug)]
pub(crate) struct RunsRead {
    pub runs: Vec<PipelineRun>,
    pub evidence: RunsEvidence,
}

/// A `pipelines` connection from [`RUNS_QUERY`].
pub fn parse_runs(conn: &Value, base: &str) -> Vec<PipelineRun> {
    nodes(conn)
        .map(|r| {
            let name = str_of(&r["user"]["name"]);
            PipelineRun {
                iid: str_of(&r["iid"]),
                status: r["status"].as_str().and_then(Pipeline::from_graphql),
                created_at: str_of(&r["createdAt"]),
                duration_secs: secs_of(&r["duration"]),
                url: url_of(&r["path"], base),
                source: str_of(&r["source"]),
                user: if name.is_empty() {
                    str_of(&r["user"]["username"])
                } else {
                    name
                },
                commit_sha: str_of(&r["commit"]["shortId"]),
                commit_title: text_of(&r["commit"]["title"]),
            }
        })
        .collect()
}

fn next_page(conn: &Value) -> Option<Value> {
    let page = &conn["pageInfo"];
    (page["hasNextPage"].as_bool() == Some(true)).then(|| page["endCursor"].clone())
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// A field GitLab's GraphQL sends HTML-escaped (a commit's title): as text.
fn text_of(v: &Value) -> String {
    let s = str_of(v);
    if !s.contains('&') {
        return s;
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        // Last, so `&amp;lt;` becomes `&lt;` and not `<`.
        .replace("&amp;", "&")
}

fn u32_of(v: &Value) -> u32 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0) as u32
}

fn usernames(v: &Value) -> Vec<String> {
    v["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| str_of(&n["username"]))
        .collect()
}

pub fn parse_mr(n: &Value, base_url: &str) -> Mr {
    let pipeline = n["headPipeline"]["status"]
        .as_str()
        .and_then(Pipeline::from_graphql);
    let pipeline_url = n["headPipeline"]["path"]
        .as_str()
        .map(|p| format!("{base_url}{p}"));
    Mr {
        iid: u32_of(&n["iid"]) as u64,
        title: str_of(&n["title"]),
        draft: n["draft"].as_bool().unwrap_or(false),
        web_url: str_of(&n["webUrl"]),
        updated_at: str_of(&n["updatedAt"]),
        source_branch: str_of(&n["sourceBranch"]),
        target_branch: str_of(&n["targetBranch"]),
        head_sha: str_of(&n["diffHeadSha"]),
        notes: u32_of(&n["userNotesCount"]),
        conflicts: n["conflicts"].as_bool().unwrap_or(false),
        merge_status: str_of(&n["detailedMergeStatus"]),
        rebase: n["shouldBeRebased"].as_bool().unwrap_or(false),
        // Only the checks that stand in the way (failed) or are not
        // settled yet (checking); None when the server has no such list.
        checks: n["mergeabilityChecks"].as_array().map(|a| {
            a.iter()
                .filter(|c| matches!(c["status"].as_str(), Some("FAILED" | "CHECKING")))
                .map(|c| crate::model::Check {
                    id: str_of(&c["identifier"]),
                    checking: c["status"].as_str() == Some("CHECKING"),
                })
                .collect()
        }),
        approved: n["approved"].as_bool().unwrap_or(false),
        approvals_required: u32_of(&n["approvalsRequired"]),
        approvals_left: u32_of(&n["approvalsLeft"]),
        author: Person {
            username: str_of(&n["author"]["username"]),
            name: str_of(&n["author"]["name"]),
        },
        reviewers: usernames(&n["reviewers"]),
        approved_by: usernames(&n["approvedBy"]),
        pipeline,
        pipeline_url,
        additions: u32_of(&n["diffStatsSummary"]["additions"]),
        deletions: u32_of(&n["diffStatsSummary"]["deletions"]),
        files: u32_of(&n["diffStatsSummary"]["fileCount"]),
        project_path: str_of(&n["project"]["fullPath"]),
        project_name: str_of(&n["project"]["name"]),
        stages: n["headPipeline"]["stages"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| Stage {
                name: str_of(&s["name"]),
                status: str_of(&s["status"]),
            })
            .collect(),
        duration_secs: n["headPipeline"]["duration"].as_u64().map(|d| d as u32),
        threads: u32_of(&n["resolvableDiscussionsCount"]),
        threads_resolved: u32_of(&n["resolvedDiscussionsCount"]),
        last_comment: n["notes"]["nodes"]
            .as_array()
            .and_then(|a| a.last())
            .map(|c| Comment {
                author: str_of(&c["author"]["name"]),
                body: str_of(&c["body"]),
                at: str_of(&c["createdAt"]),
                url: c["url"].as_str().filter(|u| !u.is_empty()).map(str::to_string),
            }),
        state: MrState::from_graphql(n["state"].as_str().unwrap_or("opened")),
        ended_at: n["mergedAt"]
            .as_str()
            .or_else(|| n["closedAt"].as_str())
            .unwrap_or_default()
            .to_string(),
        labels: nodes(&n["labels"]).map(|l| str_of(&l["title"])).collect(),
    }
}

#[cfg(test)]
pub mod recorded {
    //! Recorded GitLab replies, played back in order; the requests are kept.

    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Replay {
        pub replies: Mutex<std::collections::VecDeque<Resp>>,
        pub asked: Mutex<Vec<Req>>,
    }

    impl Replay {
        pub fn new(replies: Vec<(u16, Value)>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .map(|(status, body)| Resp {
                            status,
                            content_range: None,
                            pacing: Pacing::default(),
                            body: if body.is_null() { vec![] } else { body.to_string().into_bytes() },
                        })
                        .collect(),
                ),
                asked: Mutex::default(),
            })
        }

        /// Replies given as they came, headers and all.
        pub fn raw(replies: Vec<Resp>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                asked: Mutex::default(),
            })
        }

        pub fn asked(&self) -> Vec<Req> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Transport for Replay {
        fn send(&self, req: &Req) -> Sent<'_> {
            self.asked.lock().unwrap().push(req.clone());
            let next = self.replies.lock().unwrap().pop_front();
            Box::pin(async move { next.ok_or_else(|| "no recorded reply left".to_string()) })
        }
    }

    pub fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }
}

#[cfg(test)]
mod http_fixture;

#[cfg(test)]
mod tests {
    use super::recorded::{Replay, block};
    use super::*;

    #[test]
    fn a_page_is_asked_for_by_project_and_iid_and_read_from_the_answer() {
        let r = Replay::new(vec![(
            200,
            json!({"data": {"project": {"mergeRequest": {
                "iid": "263", "title": "Stats endpoint", "project": {"name": "api", "fullPath": "acme/team-a/api"},
                "description": "Closes ABC-12",
                "diffStats": [{"path": "src/a.rs", "additions": 3, "deletions": 1}]
            }}}}),
        )]);
        let (mr, d) = block(Client::over(r.clone()).mr_detail("acme/team-a/api", 263)).unwrap();
        assert_eq!((mr.iid, d.files[0].path.as_str()), (263, "src/a.rs"));
        let asked = r.asked();
        assert_eq!((asked[0].method, asked[0].path.as_str()), (Method::Post, "/api/graphql"));
        assert_eq!(
            asked[0].body.as_ref().unwrap()["variables"],
            json!({"project": "acme/team-a/api", "iid": "263"})
        );
    }

    #[test]
    fn graphql_errors_and_a_refused_token_are_said() {
        let r = Replay::new(vec![(200, json!({"errors": [{"message": "Field 'x' doesn't exist"}]}))]);
        let e = block(Client::over(r).mr_detail("acme/team-a/api", 1)).unwrap_err();
        assert_eq!(e.to_string(), "GitLab: Field 'x' doesn't exist");
        let r = Replay::new(vec![(401, json!({"message": "401 Unauthorized"}))]);
        let e = block(Client::over(r).mr_detail("acme/team-a/api", 1)).unwrap_err();
        assert!(is_rejected(&e));
        let r = Replay::new(vec![(404, json!({"message": "404 Job Not Found"}))]);
        let e = block(Client::over(r).job("acme/team-a/api", 9)).unwrap_err();
        assert_eq!(e.to_string(), "GitLab answered 404 Not Found · 404 Job Not Found");
    }

    /// A server that is down or limiting is a `Trouble` the sync backs off
    /// from, with what its headers said; a refused token and any other
    /// refusal are not.
    #[test]
    fn a_5xx_or_429_is_a_trouble_carrying_its_pacing_and_other_refusals_are_not() {
        use crate::backoff::trouble_of;
        let pacing = Pacing {
            retry_at: Some(1_800_000_120),
            reset_at: Some(1_800_000_060),
            remaining: Some(0),
            limit: Some(2000),
        };
        let sync = |status: u16, body: &[u8]| {
            let r = Replay::raw(vec![Resp {
                status,
                body: body.to_vec(),
                pacing,
                ..Default::default()
            }]);
            block(Client::over(r).sync("acme/team-a", &Want::default())).unwrap_err()
        };
        for status in [429, 500, 502, 503, 504] {
            let e = sync(status, b"<html>Bad Gateway</html>");
            let t = trouble_of(&e).unwrap_or_else(|| panic!("{status}: {e:#}"));
            assert_eq!((t.status, t.pacing), (Some(status), pacing));
            assert_eq!(e.to_string(), format!("GitLab answered {}", status_words(status)));
        }
        let e = sync(503, br#"{"errors":[{"message":"maintenance"}]}"#);
        assert_eq!(trouble_of(&e).map(|t| t.status), Some(Some(503)));
        assert_eq!(e.to_string(), "GitLab: maintenance");
        let e = sync(401, b"{}");
        assert!(is_rejected(&e) && trouble_of(&e).is_none());
        let e = sync(404, b"<html>gone</html>");
        assert!(trouble_of(&e).is_none(), "{e:#}");
        let e = sync(200, br#"{"errors":[{"message":"no such field"}]}"#);
        assert!(trouble_of(&e).is_none(), "{e:#}");

        let r = Replay::new(vec![(502, json!({"message": "oops"}))]);
        let e = block(Client::over(r).job("acme/team-a/api", 9)).unwrap_err();
        assert_eq!(e.to_string(), "GitLab answered 502 Bad Gateway · oops");
        assert_eq!(trouble_of(&e).map(|t| t.status), Some(Some(502)));
    }

    #[test]
    fn no_answer_at_all_is_a_trouble_and_the_latest_answers_pacing_is_kept() {
        use crate::backoff::trouble_of;
        let e = block(Client::over(Replay::raw(vec![])).sync("acme/team-a", &Want::default()))
            .unwrap_err();
        assert_eq!(e.to_string(), "no recorded reply left");
        assert_eq!(trouble_of(&e).map(|t| t.status), Some(None));

        let pacing = Pacing {
            remaining: Some(7),
            limit: Some(2000),
            ..Pacing::default()
        };
        let body = json!({"data": {"currentUser": {"username": "a", "name": "A"}}});
        let r = Replay::raw(vec![Resp {
            status: 200,
            body: body.to_string().into_bytes(),
            pacing,
            ..Default::default()
        }]);
        let client = Client::over(r);
        assert_eq!(client.pacing(), Pacing::default());
        block(client.clone().check_user()).unwrap();
        assert_eq!(client.pacing(), pacing, "shared by the clones, for the sync to read");
    }

    #[test]
    fn a_down_gitlab_is_said_by_its_status_not_by_the_json_parser() {
        let html = b"<html><body><h1>503 Service Unavailable</h1></body></html>".to_vec();
        let r = Replay::raw(vec![Resp { status: 503, body: html, ..Default::default() }]);
        let e = block(Client::over(r).mr_detail("acme/team-a/api", 1)).unwrap_err();
        assert_eq!(e.to_string(), "GitLab answered 503 Service Unavailable");
    }

    #[test]
    fn a_log_part_is_asked_for_with_its_range_and_read_with_where_it_starts() {
        let r = Replay::raw(vec![Resp {
            status: 206,
            content_range: Some("bytes 4446-4456/6446".into()),
            body: b"tail of it\n".to_vec(),
            ..Default::default()
        }]);
        let part = block(Client::over(r.clone()).trace("acme/team-a/api", 9, None)).unwrap();
        assert_eq!((part.start, part.total, part.text.as_str()), (4446, 6446, "tail of it\n"));
        let asked = r.asked();
        assert_eq!(asked[0].path, "/api/v4/projects/acme%2Fteam-a%2Fapi/jobs/9/trace");
        assert_eq!(asked[0].range.as_deref(), Some("bytes=-262144"));
    }

    /// A merge request GitLab has not checked is asked to be checked (a
    /// read with `with_merge_status_recheck`) before the sync reads the
    /// merge requests, and a refusal there does not stop the sync.
    #[test]
    fn an_unchecked_merge_request_is_asked_to_be_checked_first() {
        let empty = json!({"data": {"group": {"mergeRequests": {"nodes": [], "pageInfo": {"hasNextPage": false}}}}});
        let r = Replay::new(vec![(404, json!({"message": "404 Not Found"})), (200, empty)]);
        let want = Want {
            changed_since: Some("2026-10-01T00:00:00Z".into()),
            nudge: vec![("acme/team-a/api".into(), 201)],
            repo: Some(false),
            ..Default::default()
        };
        block(Client::over(r.clone()).sync("acme/team-a", &want)).unwrap();
        assert_eq!(
            r.asked()[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/201?with_merge_status_recheck=true"
        );
        assert_eq!(r.asked()[0].method, Method::Get, "a read, never a write");
    }

    #[test]
    fn a_files_diff_is_asked_for_a_page_at_a_time() {
        let r = Replay::new(vec![(
            200,
            json!([
                {"old_path": "src/a.rs", "new_path": "src/a.rs", "new_file": false, "renamed_file": false,
                 "deleted_file": false, "diff": "@@ -1,2 +1,2 @@\n-a\n+b\n c\n"},
                {"old_path": "old.rs", "new_path": "new.rs", "renamed_file": true, "diff": ""},
                {"old_path": "big.json", "new_path": "big.json", "diff": "", "too_large": true}
            ]),
        )]);
        let files = block(Client::over(r.clone()).diffs("acme/team-a/api", 263, 2)).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].diff.lines().count(), 4);
        assert_eq!(files[1].how().as_deref(), Some("renamed from old.rs"));
        assert!(files[2].too_large);
        assert_eq!(
            r.asked()[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/merge_requests/263/diffs?page=2&per_page=20"
        );
        assert_eq!(r.asked()[0].method, Method::Get);
    }

    #[test]
    fn a_page_carries_the_diffs_commits_what_a_merge_does_and_where_threads_are() {
        let n = json!({
            "diffRefs": {"baseSha": "b", "headSha": "h", "startSha": "s"},
            "squashOnMerge": true, "shouldRemoveSourceBranch": false, "forceRemoveSourceBranch": true,
            "discussions": {"nodes": [{"id": "gid://gitlab/Discussion/abc123", "resolvable": true, "resolved": false,
                "notes": {"nodes": [{"body": "here", "createdAt": "2026-10-06T03:00:00Z",
                    "author": {"username": "bob", "name": "Bob"},
                    "position": {"filePath": "src/a.rs", "newLine": null, "oldLine": 7}}]}}]}
        });
        let d = parse_detail(&n, "https://gitlab.com");
        let refs = d.refs.unwrap();
        assert_eq!((refs.base.as_str(), refs.head.as_str(), refs.start.as_str()), ("b", "h", "s"));
        assert!(d.merge.squash && d.merge.remove_source && d.merge.remove_forced);
        let th = &d.discussions[0];
        assert_eq!(th.rest_id(), "abc123");
        assert_eq!(th.on_line(), Some(("src/a.rs", Some(7), None)));
    }

    #[test]
    fn a_write_is_sent_once_whatever_comes_back() {
        for status in [429, 500, 502] {
            let r = Replay::new(vec![(status, json!({})), (200, json!({}))]);
            let (got, _) = block(Client::over(r.clone()).put_rest("projects/x/merge_requests/1/rebase")).unwrap();
            assert_eq!(got, status);
            assert_eq!(r.asked().len(), 1, "{status}: sent once");
        }
    }

    #[test]
    fn commit_titles_come_back_as_text_not_html() {
        assert_eq!(
            text_of(&json!("feat: search &amp; filters &lt;v2&gt; &quot;x&quot; it&#39;s")),
            "feat: search & filters <v2> \"x\" it's"
        );
        assert_eq!(text_of(&json!("a &amp;lt; b")), "a &lt; b");
        assert_eq!(text_of(&json!("plain")), "plain");
    }

    #[test]
    fn parses_a_graphql_merge_request_node() {
        let n = json!({
            "iid": "198", "title": "chore: lock file", "draft": false,
            "webUrl": "https://gitlab.com/g/ui/-/merge_requests/198", "updatedAt": "2026-10-06T03:15:53Z",
            "sourceBranch": "renovate/npm", "targetBranch": "main", "diffHeadSha": "abc123", "userNotesCount": 2,
            "conflicts": false, "detailedMergeStatus": "NEED_REBASE", "approved": true,
            "approvalsRequired": 1, "approvalsLeft": 0,
            "author": {"username": "alice", "name": "Alice Example"},
            "reviewers": {"nodes": [{"username": "me"}]},
            "approvedBy": {"nodes": [{"username": "bob"}]},
            "headPipeline": {"status": "FAILED", "path": "/g/ui/-/pipelines/7"},
            "diffStatsSummary": {"additions": 113, "deletions": 112, "fileCount": 1},
            "project": {"name": "ui", "fullPath": "g/ui"}
        });
        let mr = parse_mr(&n, "https://gitlab.com");
        assert_eq!(mr.iid, 198);
        assert_eq!(mr.head_sha, "abc123");
        assert_eq!(mr.pipeline, Some(Pipeline::Failed));
        assert_eq!(
            mr.pipeline_url.as_deref(),
            Some("https://gitlab.com/g/ui/-/pipelines/7")
        );
        assert_eq!(mr.reviewers, vec!["me"]);
        assert_eq!(mr.approved_by, vec!["bob"]);
        assert_eq!((mr.additions, mr.deletions, mr.files), (113, 112, 1));
        assert_eq!(mr.project_path, "g/ui");
    }

    #[test]
    fn parses_a_merge_request_page_leaving_out_system_notes() {
        let n = json!({
            "description": "Closes ABC-1",
            "diffStats": [{"path": "src/a.rs", "additions": 3, "deletions": 1}],
            "commits": {"nodes": [{"shortId": "abc123", "title": "fix", "authorName": "Alice",
                "authoredDate": "2026-10-06T03:15:53Z", "webUrl": "https://gitlab.com/g/a/-/commit/abc123"}]},
            "jobs": {"iid": "7", "stages": {"nodes": [{"name": "test", "status": "FAILED",
                "jobs": {"nodes": [{"name": "unit", "status": "FAILED", "duration": 31,
                    "webPath": "/g/a/-/jobs/9", "allowFailure": true}]}}]}},
            "discussions": {"nodes": [
                {"resolvable": false, "resolved": false, "notes": {"nodes": [
                    {"body": "added 2 commits", "system": true, "createdAt": "2026-10-06T03:00:00Z",
                     "author": {"username": "alice", "name": "Alice"}}]}},
                {"resolvable": true, "resolved": false, "notes": {"nodes": [
                    {"body": "why?", "system": false, "createdAt": "2026-10-06T03:00:00Z",
                     "author": {"username": "bob", "name": ""},
                     "position": {"filePath": "src/a.rs", "newLine": 21, "oldLine": null}},
                    {"body": "because", "system": false, "createdAt": "2026-10-06T03:10:00Z",
                     "author": {"username": "alice", "name": "Alice"}, "position": null}]}}
            ]}
        });
        let d = parse_detail(&n, "https://gitlab.com");
        assert_eq!(d.files[0].additions, 3);
        assert_eq!(d.commits[0].short_id, "abc123");
        let pl = d.pipeline.unwrap();
        assert_eq!(pl.stages[0].status, "failed");
        assert_eq!(
            pl.stages[0].jobs[0].url.as_deref(),
            Some("https://gitlab.com/g/a/-/jobs/9")
        );
        assert!(pl.stages[0].jobs[0].allow_failure);
        assert_eq!(d.discussions.len(), 1, "the system-only one is left out");
        let th = &d.discussions[0];
        assert!(th.is_open());
        assert_eq!(th.notes[0].author, "bob", "username when there is no name");
        assert_eq!(th.place().as_deref(), Some("a.rs:21"));
        assert_eq!(th.notes.len(), 2);
    }

    #[test]
    fn reads_a_failed_job_from_the_jobs_api() {
        let j: Value = serde_json::from_str(
            r#"{"id": 9, "name": "deploy-test", "stage": "deploy", "status": "failed",
                "failure_reason": "script_failure", "duration": 33.02,
                "web_url": "https://gitlab.com/g/a/-/jobs/9", "allow_failure": false,
                "pipeline": {"id": 77}}"#,
        )
        .unwrap();
        let job = parse_job(&j, "g/a");
        assert_eq!(
            (job.id, job.stage.as_str(), job.duration_secs, job.pipeline),
            (9, "deploy", Some(33), 77)
        );
        assert_eq!(job.failure_reason, "script_failure");
        assert_eq!(encoded("g/sub/a"), "g%2Fsub%2Fa");
        assert_eq!(job_id_of("/g/a/-/jobs/9"), Some(9));
        assert_eq!(pipeline_id_of("https://gitlab.com/g/a/-/pipelines/77?x=1"), Some(77));
        assert_eq!(job_id_of("/g/a/-/pipelines/7"), None);
        assert_eq!(
            rest_message(&serde_json::json!({"message": "404 Job Not Found"})),
            " · 404 Job Not Found"
        );
    }

    #[test]
    fn a_log_is_asked_for_from_its_end_and_its_place_read_from_the_answer() {
        assert_eq!(trace_range(None), "bytes=-262144");
        assert_eq!(trace_range(Some(300_000)), "bytes=37856-299999");
        assert_eq!(trace_range(Some(100)), "bytes=0-99");
        let part = trace_part(b"tail of it\n", Some("bytes 4446-4456/6446"));
        assert_eq!((part.start, part.total), (4446, 6446));
        // No range in the answer: the whole log.
        let whole = trace_part("all of it ✓\n".as_bytes(), None);
        assert_eq!((whole.start, whole.total), (0, 14));
        assert_eq!(whole.text, "all of it ✓\n");
    }

    #[test]
    fn parses_pipeline_runs() {
        let conn = json!({"nodes": [{"iid": "12", "status": "SUCCESS", "createdAt": "2026-10-06T03:00:00Z",
            "duration": 82, "path": "/g/a/-/pipelines/5", "source": "schedule",
            "user": {"name": "Bob", "username": "bob"}, "commit": {"shortId": "0110f094", "title": "chore"}}]});
        let runs = parse_runs(&conn, "https://gitlab.com");
        assert_eq!(runs[0].status, Some(Pipeline::Passed));
        assert_eq!(
            runs[0].url.as_deref(),
            Some("https://gitlab.com/g/a/-/pipelines/5")
        );
        assert_eq!(runs[0].commit_sha, "0110f094");
    }

    #[test]
    fn merged_and_closed_states_labels_and_when_they_ended() {
        let base = "https://gitlab.com";
        let merged = parse_mr(
            &json!({"iid": 2, "state": "merged", "mergedAt": "2026-10-05T07:16:41Z",
                    "closedAt": null, "labels": {"nodes": [{"title": "dependencies"}, {"title": "type::bug"}]}}),
            base,
        );
        assert_eq!(merged.state, MrState::Merged);
        assert_eq!(merged.ended_at, "2026-10-05T07:16:41Z");
        assert_eq!(merged.labels, vec!["dependencies", "type::bug"]);
        let closed = parse_mr(
            &json!({"iid": 3, "state": "locked", "closedAt": "2026-10-04T00:00:00Z"}),
            base,
        );
        assert_eq!(closed.state, MrState::Closed);
        assert_eq!(closed.ended_at, "2026-10-04T00:00:00Z");
        let open = parse_mr(&json!({"iid": 4}), base);
        assert_eq!((open.state, open.labels.len()), (MrState::Opened, 0));
    }

    #[test]
    fn a_project_carries_its_main_branch_commit_and_archived_ones_are_left_out() {
        let n = json!({"name": "api", "fullPath": "acme/team-a/api", "archived": false,
            "lastActivityAt": "2026-10-06T00:00:00Z",
            "repository": {"rootRef": "main", "tree": {"lastCommit": {"shortId": "a22afd6f",
                "sha": "a22afd6f1c0de5b7e9c4a3d2f1b0a9c8d7e6f5a4",
                "authorName": "Alice Example", "authoredDate": "2026-08-31T11:10:14+02:00",
                "title": "feat: search &amp; filters"}}},
            "pipelines": {"nodes": [
                {"id": "gid://gitlab/Ci::Pipeline/501", "ref": "feature/x", "status": "FAILED", "path": "/p/1",
                 "sha": "ffff", "updatedAt": "2026-10-06T00:00:00Z"},
                {"id": "gid://gitlab/Ci::Pipeline/498", "ref": "main", "status": "SUCCESS", "path": "/p/2",
                 "sha": "a22afd6f1c0de5b7e9c4a3d2f1b0a9c8d7e6f5a4", "updatedAt": "2026-10-05T12:30:00Z"}]}});
        let pr = parse_project(&n, "https://gitlab.com").unwrap();
        assert_eq!(pr.main_pipeline, Some(Pipeline::Passed));
        assert_eq!(pr.main_pipeline_url.as_deref(), Some("https://gitlab.com/p/2"));
        assert_eq!(pr.main_pipeline_id, Some(498), "main's, not the feature branch's");
        assert_eq!(pr.main_pipeline_at, "2026-10-05T12:30:00Z");
        let reference = pr.pin().expect("a finished pipeline and a head commit");
        assert_eq!(reference.head_sha, "a22afd6f1c0de5b7e9c4a3d2f1b0a9c8d7e6f5a4");
        assert_eq!((reference.branch.as_str(), reference.pipeline_id), ("main", Some(498)));
        let c = pr.main_commit.unwrap();
        assert_eq!(c.sha, "a22afd6f1c0de5b7e9c4a3d2f1b0a9c8d7e6f5a4");
        assert_eq!((c.short_id.as_str(), c.author.as_str()), ("a22afd6f", "Alice Example"));
        assert_eq!(c.title, "feat: search & filters");
        let empty = json!({"name": "new", "fullPath": "acme/new", "repository": {"rootRef": null, "tree": null}});
        assert_eq!(parse_project(&empty, "").unwrap().main_commit, None);
        assert!(parse_project(&json!({"archived": true}), "").is_none());
    }

    #[test]
    fn a_rejected_token_is_recognised_through_context() {
        let e = anyhow::Error::from(Rejected).context("syncing acme");
        assert!(is_rejected(&e));
        assert!(!is_rejected(&anyhow::anyhow!("GitLab unreachable")));
        assert_eq!(Rejected.to_string(), "GitLab rejected the token (401)");
    }

    #[test]
    fn the_list_query_reads_a_group_with_subgroups_or_one_project() {
        let g = mrs_query(false, PAGE);
        assert!(g.contains("group(fullPath: $path)") && g.contains("includeSubgroups: true"));
        let p = mrs_query(true, PAGE);
        assert!(p.contains("project(fullPath: $path)") && !p.contains("includeSubgroups"));
        assert!(p.contains("labels { nodes { title } }") && p.contains("updatedAfter: $updatedAfter"));
        assert!(p.contains("first: 50"));
    }

    fn no_changes() -> Value {
        json!({"data": {"group": {"mergeRequests": {"nodes": [], "pageInfo": {"hasNextPage": false}}}}})
    }

    /// iids are not tied to a project: asked by iids across a group, GitLab
    /// finds each in every project (47 KB for 2). The recheck asks each
    /// merge request of its own project, a few to a query, for only its
    /// pipeline and merge status.
    #[test]
    fn a_recheck_asks_each_merge_request_of_its_own_project() {
        let answer = json!({"data": {
            "m0": {"mergeRequest": {"detailedMergeStatus": "MERGEABLE", "conflicts": false,
                "shouldBeRebased": false, "mergeabilityChecks": [],
                "headPipeline": {"status": "SUCCESS", "path": "/acme/team-a/api/-/pipelines/9",
                    "duration": 61, "stages": {"nodes": [{"name": "test", "status": "success"}]}}}},
            "m1": {"mergeRequest": null}
        }});
        let empty = json!({"data": {}});
        let r = Replay::new(vec![
            (200, no_changes()),
            (200, answer),
            (200, empty.clone()),
            (200, empty.clone()),
            (200, empty),
        ]);
        let mut recheck: Vec<(String, u64)> =
            (1..=25).map(|i| ("acme/team-a/api".to_string(), i)).collect();
        recheck[1].0 = "acme/team-a/web".into();
        let want = Want {
            changed_since: Some("2026-10-01T00:00:00Z".into()),
            recheck,
            repo: Some(false),
            ..Default::default()
        };
        let got = block(Client::over(r.clone()).sync("acme/team-a", &want)).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 5, "the changes, then 25 rechecks 8 at a time");
        let body = |i: usize| asked[i].body.clone().unwrap();
        let q = body(1)["query"].as_str().unwrap().to_string();
        assert!(q.contains("m0: project(fullPath: $p0) { mergeRequest(iid: $i0) {"), "{q}");
        assert!(!q.contains("iids") && !q.contains("group("), "{q}");
        assert!(!q.contains("title") && q.contains("detailedMergeStatus") && q.contains("headPipeline"));
        assert_eq!(body(1)["variables"]["p0"], "acme/team-a/api");
        assert_eq!(body(1)["variables"]["i0"], "1");
        assert_eq!(body(1)["variables"]["i1"], "3");
        // web's !2 comes last, in its own project.
        assert_eq!(body(4)["variables"]["p0"], "acme/team-a/web");
        assert_eq!(body(4)["variables"]["i0"], "2");
        assert!(body(4)["variables"]["p1"].is_null());
        assert_eq!(got.rechecked.len(), 1, "one GitLab no longer shows is left out");
        let m = &got.rechecked[0];
        assert_eq!((m.project_path.as_str(), m.iid), ("acme/team-a/api", 1));
        assert_eq!((m.pipeline, m.merge_status.as_str()), (Some(Pipeline::Passed), "MERGEABLE"));
        assert_eq!(m.duration_secs, Some(61));
        assert!(!got.mrs_truncated);
    }

    fn activity(rows: &[(&str, &str)]) -> Value {
        let nodes: Vec<Value> = rows
            .iter()
            .map(|(p, at)| json!({"fullPath": p, "archived": false, "lastActivityAt": at}))
            .collect();
        json!({"data": {"group": {"projects": {"nodes": nodes, "pageInfo": {"hasNextPage": false}}}}})
    }

    fn check(seen: &[(&str, &str)], again: &[&str]) -> Want {
        Want {
            changed_since: Some("2026-10-01T00:00:00Z".into()),
            projects: true,
            projects_check: Some(ProjectsCheck {
                seen: seen.iter().map(|(p, a)| (p.to_string(), a.to_string())).collect(),
                again: again.iter().map(|p| p.to_string()).collect(),
                ..Default::default()
            }),
            repo: Some(false),
            ..Default::default()
        }
    }

    const T: &str = "2026-10-06T00:00:00Z";

    /// A quiet project (its activity where it was) costs only the cheap
    /// check: its commit and pipelines are not asked again.
    #[test]
    fn a_quiet_projects_commit_and_pipelines_are_not_asked_again() {
        let r = Replay::new(vec![(200, no_changes()), (200, activity(&[("acme/api", T), ("acme/web", T)]))]);
        let want = check(&[("acme/api", T), ("acme/web", T)], &[]);
        let got = block(Client::over(r.clone()).sync("acme", &want)).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 2, "the changes and the activity, nothing more");
        let q = asked[1].body.as_ref().unwrap()["query"].as_str().unwrap().to_string();
        assert!(q.contains("lastActivityAt") && !q.contains("lastCommit") && !q.contains("pipelines"));
        let seen = got.projects_seen.unwrap();
        assert!(seen.fresh.is_empty());
        assert_eq!(seen.present.len(), 2);
        assert_eq!(got.projects, None, "not fetched in full");
    }

    /// Activity moved, a new project, one with main's pipeline running
    /// (`again`) and one a merge request merged into: those, and only
    /// those, are fetched in full, each by its path.
    #[test]
    fn projects_that_moved_or_whose_main_pipeline_runs_are_fetched_in_full() {
        let merged = json!({"data": {"group": {"mergeRequests": {"nodes": [{"iid": 5, "state": "merged",
            "project": {"fullPath": "acme/docs", "name": "docs"}}], "pageInfo": {"hasNextPage": false}}}}});
        let later = "2026-10-06T09:00:00Z";
        let rows = [("acme/api", later), ("acme/docs", T), ("acme/new", T), ("acme/quiet", T), ("acme/ui", T)];
        let full = json!({"data": {"p0": {"name": "api", "fullPath": "acme/api", "lastActivityAt": later,
            "repository": {"rootRef": "main"}, "pipelines": {"nodes": [{"ref": "main", "status": "RUNNING"}]}}}});
        let r = Replay::new(vec![(200, merged), (200, activity(&rows)), (200, full)]);
        let want = check(&[("acme/api", T), ("acme/docs", T), ("acme/quiet", T), ("acme/ui", T)], &["acme/ui"]);
        let got = block(Client::over(r.clone()).sync("acme", &want)).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 3);
        let body = asked[2].body.as_ref().unwrap();
        assert!(body["query"].as_str().unwrap().contains("p0: project(fullPath: $p0)"));
        assert!(body["query"].as_str().unwrap().contains("lastCommit"));
        let paths: Vec<&str> = (0..5).filter_map(|i| body["variables"][format!("p{i}")].as_str()).collect();
        assert_eq!(paths, ["acme/api", "acme/docs", "acme/new", "acme/ui"], "not acme/quiet");
        let seen = got.projects_seen.unwrap();
        assert_eq!(seen.fresh.len(), 1);
        assert_eq!(seen.fresh[0].main_pipeline, Some(Pipeline::Running));
    }

    fn reference(head: &str, id: u64) -> MainRef {
        MainRef {
            branch: "main".into(),
            head_sha: head.into(),
            pipeline_id: Some(id),
            pipeline: Some(Pipeline::Passed),
            pipeline_at: "2026-10-05T12:30:00Z".into(),
        }
    }

    /// What GitLab says to a pin query of one project: its branch, head
    /// commit, and newest pipeline on it.
    fn pinned(head: &str, id: u64) -> Value {
        json!({"repository": {"rootRef": "main", "tree": {"lastCommit": {"sha": head}}},
            "pipelines": {"nodes": [{"id": format!("gid://gitlab/Ci::Pipeline/{id}"),
                "status": "SUCCESS", "updatedAt": "2026-10-05T12:30:00Z"}]}})
    }

    /// A project to look at with a reference: `moved` its activity.
    fn looked_at(seen_at: &str, recent: bool) -> Want {
        let mut want = check(&[("acme/api", seen_at)], &[]);
        let c = want.projects_check.as_mut().unwrap();
        c.pins.insert("acme/api".into(), reference("aaa111", 70));
        if recent {
            c.look.insert("acme/api".into());
        }
        want
    }

    /// Active within the hour GitLab's activity does not move, so the
    /// project is looked at every time; its reference (branch, head commit,
    /// newest pipeline) is asked for it, a fraction of the full fetch, and
    /// when it is where it was the project is not fetched in full.
    #[test]
    fn a_recent_project_is_pinpointed_by_its_reference_not_fetched_in_full() {
        let r = Replay::new(vec![
            (200, no_changes()),
            (200, activity(&[("acme/api", T)])),
            (200, json!({"data": {"p0": pinned("aaa111", 70)}})),
        ]);
        let got = block(Client::over(r.clone()).sync("acme", &looked_at(T, true))).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 3, "the changes, the activity, the reference");
        let body = asked[2].body.as_ref().unwrap();
        let q = body["query"].as_str().unwrap();
        assert!(q.contains("p0: project(fullPath: $p0)") && q.contains("sha"));
        assert!(
            !q.contains("authorName") && !q.contains("first: 10") && !q.contains("title"),
            "not the full fetch: {q}"
        );
        assert_eq!(body["variables"]["p0"], "acme/api");
        assert_eq!(body["variables"]["r0"], "main", "the pipelines of its branch");
        let seen = got.projects_seen.unwrap();
        assert!(seen.fresh.is_empty(), "main has not moved");
        assert_eq!(seen.present, [("acme/api".to_string(), T.to_string())]);
    }

    /// A push to main inside the hour shows in the head commit and the
    /// pipeline: then, and only then, the project is fetched in full.
    #[test]
    fn a_pinpointed_project_whose_head_or_pipeline_moved_is_fetched_in_full() {
        for (head, id) in [("bbb222", 70), ("aaa111", 71)] {
            let full = json!({"data": {"p0": {"name": "api", "fullPath": "acme/api", "lastActivityAt": T,
                "repository": {"rootRef": "main"}, "pipelines": {"nodes": []}}}});
            let r = Replay::new(vec![
                (200, no_changes()),
                (200, activity(&[("acme/api", T)])),
                (200, json!({"data": {"p0": pinned(head, id)}})),
                (200, full),
            ]);
            let got = block(Client::over(r.clone()).sync("acme", &looked_at(T, true))).unwrap();
            let asked = r.asked();
            assert_eq!(asked.len(), 4, "{head} {id}: the reference differs: in full");
            assert!(asked[3].body.as_ref().unwrap()["query"].as_str().unwrap().contains("authorName"));
            assert_eq!(got.projects_seen.unwrap().fresh.len(), 1);
        }
    }

    /// Activity moves for any branch: a push to a feature branch moves it,
    /// and main (head, pipeline) is where it was, so no full fetch.
    #[test]
    fn a_project_whose_activity_moved_but_whose_main_did_not_is_not_fetched_in_full() {
        let later = "2026-10-06T09:00:00Z";
        let r = Replay::new(vec![
            (200, no_changes()),
            (200, activity(&[("acme/api", later)])),
            (200, json!({"data": {"p0": pinned("aaa111", 70)}})),
        ]);
        let got = block(Client::over(r.clone()).sync("acme", &looked_at(T, false))).unwrap();
        assert_eq!(r.asked().len(), 3);
        let seen = got.projects_seen.unwrap();
        assert!(seen.fresh.is_empty());
        assert_eq!(seen.present[0].1, later, "its new activity is what is held");
    }

    /// A pipeline still running on main is pinpointed too: while it runs
    /// nothing about the project has changed, and its end (the state
    /// differs) is what brings the full fetch.
    #[test]
    fn a_running_pipeline_is_fetched_in_full_when_it_ends_not_while_it_runs() {
        let running = |status: &str| {
            let mut n = pinned("aaa111", 70);
            n["pipelines"]["nodes"][0]["status"] = json!(status);
            json!({"data": {"p0": n}})
        };
        let full = json!({"data": {"p0": {"name": "api", "fullPath": "acme/api"}}});
        for (status, asked) in [("RUNNING", 3), ("SUCCESS", 4)] {
            let r = Replay::new(vec![
                (200, no_changes()),
                (200, activity(&[("acme/api", T)])),
                (200, running(status)),
                (200, full.clone()),
            ]);
            let mut want = looked_at(T, true);
            let c = want.projects_check.as_mut().unwrap();
            c.pins.get_mut("acme/api").unwrap().pipeline = Some(Pipeline::Running);
            block(Client::over(r.clone()).sync("acme", &want)).unwrap();
            assert_eq!(r.asked().len(), asked, "{status}");
        }
    }

    /// A quiet project (activity where it was, not recent) is not looked at
    /// at all; one with nothing to compare with is fetched in full at once,
    /// as before; so is one whose main pipeline runs.
    #[test]
    fn only_projects_with_a_reference_are_pinpointed() {
        let rows = [("acme/api", T), ("acme/old", T), ("acme/run", T), ("acme/quiet", T)];
        let full = json!({"data": {"p0": {"name": "old", "fullPath": "acme/old"},
            "p1": {"name": "run", "fullPath": "acme/run"}}});
        let r = Replay::new(vec![
            (200, no_changes()),
            (200, activity(&rows)),
            (200, json!({"data": {"p0": pinned("aaa111", 70)}})),
            (200, full),
        ]);
        let mut want = looked_at(T, true);
        let c = want.projects_check.as_mut().unwrap();
        for p in ["acme/old", "acme/run", "acme/quiet"] {
            c.seen.insert(p.into(), T.into());
        }
        c.again.extend(["acme/old".to_string(), "acme/run".to_string()]);
        c.pins.insert("acme/quiet".into(), reference("ccc333", 5));
        let got = block(Client::over(r.clone()).sync("acme", &want)).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 4);
        let pin = asked[2].body.as_ref().unwrap();
        assert_eq!(pin["variables"]["p0"], "acme/api");
        assert!(pin["variables"]["p1"].is_null(), "only the one with a reference and recent");
        let full = asked[3].body.as_ref().unwrap();
        assert_eq!(full["variables"]["p0"], "acme/old");
        assert_eq!(full["variables"]["p1"], "acme/run");
        assert!(full["variables"]["p2"].is_null());
        assert_eq!(got.projects_seen.unwrap().fresh.len(), 2);
    }

    /// Projects fetched in full go at most 6 to a query: 213 (measured) of
    /// the 250 GitLab allows.
    #[test]
    fn projects_fetched_in_full_go_six_to_a_query() {
        let rows: Vec<(String, &str)> = (0..8).map(|i| (format!("acme/p{i}"), T)).collect();
        let rows: Vec<(&str, &str)> = rows.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let empty = json!({"data": {}});
        let r = Replay::new(vec![(200, no_changes()), (200, activity(&rows)), (200, empty.clone()), (200, empty)]);
        block(Client::over(r.clone()).sync("acme", &check(&[], &[]))).unwrap();
        let asked = r.asked();
        assert_eq!(asked.len(), 4, "the changes, the activity, then 8 new ones 6 and 2");
        assert!(asked[2].body.as_ref().unwrap()["variables"]["p5"].is_string());
        assert!(asked[3].body.as_ref().unwrap()["variables"]["p2"].is_null());
    }

    #[test]
    fn a_missing_pipeline_is_none() {
        let mr = parse_mr(
            &json!({"iid": 1, "headPipeline": null}),
            "https://gitlab.com",
        );
        assert_eq!(mr.pipeline, None);
        assert_eq!(mr.pipeline_url, None);
    }

    /// Counts what a client sends and gets, over a real transport.
    struct Counting {
        inner: Arc<dyn Transport>,
        sent: Mutex<(usize, usize, usize)>,
    }

    impl Transport for Counting {
        fn send(&self, req: &Req) -> Sent<'_> {
            let out = req.body.as_ref().map_or(0, |b| b.to_string().len());
            let req = req.clone();
            Box::pin(async move {
                let got = self.inner.send(&req).await?;
                let mut s = self.sent.lock().unwrap();
                *s = (s.0 + 1, s.1 + out, s.2 + got.body.len());
                Ok(got)
            })
        }
    }

    /// Live and read-only (queries only), few and spaced: what a quiet tick
    /// of the projects check costs on a real group, the reference as it was
    /// (every project active within the hour in full) against pinpointing
    /// by it. `RADAR_GITLAB_LIVE_GROUPS` is a comma-separated list of groups,
    /// `GITLAB_TOKEN` the token, `GITLAB_URL` the server.
    /// `cargo test -p radar-gitlab --lib live_quiet_tick -- --ignored --nocapture`
    #[test]
    #[ignore = "live: reads GitLab"]
    fn live_quiet_tick_before_and_after_pinpointing() {
        let token = std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN");
        let url = std::env::var("GITLAB_URL").unwrap_or_else(|_| "https://gitlab.com".into());
        let groups = std::env::var("RADAR_GITLAB_LIVE_GROUPS").expect("RADAR_GITLAB_LIVE_GROUPS");
        for group in groups.split(',') {
            let real = Client::new(&url, token.clone());
            let counting = Arc::new(Counting { inner: real.t.clone(), sent: Mutex::new((0, 0, 0)) });
            let client = Client::over(counting.clone());
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let held = rt.block_on(client.projects(group)).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            let data = GroupData { projects: held.clone(), ..Default::default() };
            let after = crate::sync::projects_check(&data, now).unwrap();
            // As before: the ones with a reference were in `again`.
            let mut before = after.clone();
            before.again.extend(std::mem::take(&mut before.look));
            before.pins.clear();
            for p in held.iter().filter(|p| p.pin().is_none()) {
                println!(
                    "  no reference: {} pipeline {:?} id {:?} commit {}",
                    p.full_path,
                    p.main_pipeline,
                    p.main_pipeline_id,
                    p.main_commit.as_ref().map_or("none", |c| if c.sha.is_empty() { "no sha" } else { "sha" })
                );
            }
            let mut report = Vec::new();
            for (name, check) in [("before", before), ("after", after.clone())] {
                *counting.sent.lock().unwrap() = (0, 0, 0);
                let want = Want {
                    changed_since: Some(radar_core::time::to_rfc3339(now - 300)),
                    projects: true,
                    projects_check: Some(check),
                    repo: Some(false),
                    ..Default::default()
                };
                let got = rt.block_on(client.sync(group, &want)).unwrap();
                let (n, out, inn) = *counting.sent.lock().unwrap();
                let seen = got.projects_seen.unwrap();
                report.push(format!("{name}: {n} requests, {out} B sent, {inn} B received, {} in full", seen.fresh.len()));
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            println!(
                "{group}: {} projects, {} looked at by a reference, {} pinned, {} in full: {}",
                held.len(),
                after.look.len(),
                after.pins.len(),
                after.again.len(),
                report.join(" | ")
            );
        }
    }
}

/// Exact first-page requests for a new group, with open MRs and projects.
/// This fixture recipe uses the real query definitions, not response shortcuts.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn initial_group_requests(path: &str) -> Vec<Req> {
    [
        (PATH_QUERY.to_owned(), json!({"path": path})),
        (mrs_query(false, PAGE), json!({"path": path, "state": "opened", "after": null, "updatedAfter": null, "iids": null})),
        (PROJECTS_QUERY.to_owned(), json!({"path": path, "after": null})),
    ].into_iter().map(|(query, variables)| Req::new(Method::Post, "/api/graphql", Some(json!({"query": query, "variables": variables})))).collect()
}
