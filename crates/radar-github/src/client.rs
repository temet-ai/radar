//! GitHub.com GET-only transport. Response URLs are never request destinations.
//!
//! Semantics: https://docs.github.com/en/rest/pulls/pulls
//! https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference
//! https://docs.github.com/en/rest/commits/statuses#list-commit-statuses-for-a-reference
//! https://docs.github.com/en/rest/using-the-rest-api/using-pagination-in-the-rest-api

pub use crate::model::{ApiError, ApiErrorKind};
use crate::{auth, config::Endpoint, model::*};
use reqwest::header::{
    AUTHORIZATION, CONTENT_TYPE, ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    LAST_MODIFIED, LINK,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::{
    collections::{BTreeSet, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

mod actions;

/// Repositories asked in one GraphQL request; each costs one connection of
/// one node, so a batch stays at one rate-limit point.
const PROBE_BATCH: usize = 50;
/// Probe batches in flight at once (76 repositories are two of them, or
/// three when none is held yet).
const PROBE_CONCURRENCY: usize = 4;
/// Repositories asked in full in one GraphQL request. Each brings its pull
/// requests, so the answer is slower and larger than one of markers: smaller
/// batches, in flight together, draw sooner than one slow big one.
const FULL_BATCH: usize = 25;
/// Open pull requests a full GraphQL read takes for a repository; one with
/// more is read over REST, which pages.
const FULL_PULLS: usize = 30;
const API_VERSION: &str = "2022-11-28";
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 5;
const MAX_REPOSITORIES: usize = crate::config::MAX_REPOSITORIES;
const PAGE_CAP: &str = "GitHub page limit reached; more results may exist";
const CHANGED_PAGES: &str =
    "GitHub results changed or pagination was incomplete; refresh to check again";
const INVALID_PAGINATION: &str = "GitHub pagination could not be safely continued";

#[derive(Clone)]
pub struct Client {
    repositories: BTreeSet<RepositoryId>,
    token_ref: String,
    /// Where this client's requests go, from `[github] api`. A request with a
    /// body goes only to its GraphQL address: a read-only query built here,
    /// never a mutation and never a URL from a response.
    api: Endpoint,
    transport: Arc<dyn Transport>,
    // Only one detail target is active per client. A clone preserves this proof.
    verified_head: Arc<Mutex<Option<HeadProof>>>,
    verified_run: Arc<Mutex<Option<actions::RunProof>>>,
    traffic: Arc<Mutex<Traffic>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HeadProof {
    base: RepositoryId,
    number: u64,
    head: Option<RepositoryId>,
    sha: String,
}

impl From<&PullRequest> for HeadProof {
    fn from(pull: &PullRequest) -> Self {
        Self {
            base: pull.repository.clone(),
            number: pull.number,
            head: pull.head_repository.clone(),
            sha: pull.head_sha.clone(),
        }
    }
}

impl Client {
    /// A client for the API at `api` (`[github] api`, validated by
    /// [`crate::config::Settings::validated`]).
    pub fn new(
        repositories: &[RepositoryId],
        token_ref: &str,
        api: &str,
    ) -> Result<Self, ApiError> {
        let api = Endpoint::parse(api).map_err(|_| error(ApiErrorKind::InvalidTarget))?;
        let transport = NativeTransport::new(&api)?;
        Self::with_endpoint(repositories, token_ref, api, Arc::new(transport))
    }

    #[cfg(test)]
    fn with_transport(
        repositories: &[RepositoryId],
        token_ref: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, ApiError> {
        Self::with_endpoint(repositories, token_ref, Endpoint::default(), transport)
    }

    fn with_endpoint(
        repositories: &[RepositoryId],
        token_ref: &str,
        api: Endpoint,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, ApiError> {
        auth::validate_reference(token_ref)?;
        if repositories.len() > MAX_REPOSITORIES {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        let repositories = repositories
            .iter()
            .map(RepositoryId::validated)
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(Self {
            repositories,
            token_ref: token_ref.into(),
            api,
            transport,
            verified_head: Arc::new(Mutex::new(None)),
            verified_run: Arc::new(Mutex::new(None)),
            traffic: Arc::new(Mutex::new(Traffic::default())),
        })
    }

    /// What this client has sent so far, and GitHub's last rate-limit reports.
    pub fn traffic(&self) -> Traffic {
        self.traffic.lock().map(|t| t.clone()).unwrap_or_default()
    }

    fn note(&self, response: &Response) {
        let Ok(mut traffic) = self.traffic.lock() else {
            return;
        };
        traffic.requests += 1;
        if response.status == 304 {
            traffic.not_modified += 1;
        }
        if let Some(rate) = rate_limit(&response.headers) {
            traffic.note(rate);
        }
    }

    fn allowed(&self, repository: &RepositoryId) -> Result<RepositoryId, ApiError> {
        let id = repository.validated()?;
        if self.repositories.contains(&id) {
            Ok(id)
        } else {
            Err(error(ApiErrorKind::InvalidTarget))
        }
    }

    /// Check the configured credential without requiring a repository.
    /// A public connection checks API availability; a token checks the
    /// authenticated account and returns its GitHub login.
    pub async fn check_connection(&self) -> Result<Option<String>, ApiError> {
        Ok(self.check_connection_scopes().await?.0)
    }

    /// [`Client::check_connection`], and the token's OAuth scopes when
    /// GitHub sends them (`X-OAuth-Scopes`, for a classic token or gh's);
    /// `None` for a fine-grained token or public access, whose permissions
    /// GitHub does not tell.
    pub async fn check_connection_scopes(&self) -> Result<(Option<String>, Option<Vec<String>>), ApiError> {
        if self.token_ref.is_empty() {
            let (value, _) = self.read::<serde_json::Value>("/rate_limit", "").await?;
            if !value
                .get("resources")
                .is_some_and(serde_json::Value::is_object)
            {
                return Err(error(ApiErrorKind::InvalidResponse));
            }
            return Ok((None, None));
        }
        let (account, headers) = self.read::<RawAccount>("/user", "").await?;
        if account.id == 0
            || account.login.is_empty()
            || account.login.len() > 39
            || account.login.starts_with('-')
            || account.login.ends_with('-')
            || !account
                .login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(error(ApiErrorKind::InvalidResponse));
        }
        Ok((Some(account.login), oauth_scopes(&headers)))
    }

    /// Repositories this authenticated token can access, in bounded pages.
    pub async fn accessible_repositories(&self) -> Result<Collection<RepositoryId>, ApiError> {
        if self.token_ref.is_empty() {
            return Err(error(ApiErrorKind::MissingToken));
        }
        self.collect::<RawRepository, _, _>(
            "/user/repos",
            "sort=full_name&direction=asc",
            None,
            |raw| {
                let key = raw
                    .id
                    .filter(|id| *id > 0)
                    .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?;
                Ok((key, raw.identity()?))
            },
        )
        .await
    }

    pub async fn repository(&self, repository: &RepositoryId) -> Result<Repository, ApiError> {
        match self.repository_if(repository, None).await? {
            Fresh::New(repository, _) => Ok(repository),
            Fresh::Same => Err(error(ApiErrorKind::InvalidResponse)),
        }
    }

    /// The repository, asked with what describes the copy held: `Same` when
    /// GitHub answers 304.
    pub async fn repository_if(
        &self,
        repository: &RepositoryId,
        known: Option<&Validator>,
    ) -> Result<Fresh<Repository, Option<Validator>>, ApiError> {
        let id = self.allowed(repository)?;
        let Some((raw, headers)) = self
            .read_if::<RawRepository>(&format!("/repos/{}", id.full_name()), "", known)
            .await?
        else {
            return Ok(Fresh::Same);
        };
        if raw.identity()? != id {
            return Err(error(ApiErrorKind::InvalidResponse));
        }
        let repository = Repository {
            id,
            description: raw.description,
            default_branch: raw
                .default_branch
                .filter(|branch| !branch.is_empty())
                .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?,
            private: raw
                .private
                .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?,
            archived: raw
                .archived
                .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?,
        };
        Ok(Fresh::New(repository, validator(&headers)))
    }

    pub async fn open_pulls(
        &self,
        repository: &RepositoryId,
    ) -> Result<Collection<PullRequest>, ApiError> {
        match self.open_pulls_if(repository, &[]).await? {
            Fresh::New(pulls, _) => Ok(pulls),
            Fresh::Same => Err(error(ApiErrorKind::InvalidResponse)),
        }
    }

    /// Open pull requests, each page asked with the validator of the page
    /// held (`known`, one per page of a complete read): every page a 304 is
    /// `Same`, with nothing parsed.
    pub async fn open_pulls_if(
        &self,
        repository: &RepositoryId,
        known: &[Validator],
    ) -> Result<Fresh<Collection<PullRequest>>, ApiError> {
        let id = self.allowed(repository)?;
        let path = format!("/repos/{}/pulls", id.full_name());
        self.collect_if(
            &path,
            "state=open&sort=updated&direction=desc",
            None,
            known,
            |raw: RawPull| {
                let pull = raw.checked(&id, None)?;
                if pull.state != "open" || pull.merged {
                    return Err(error(ApiErrorKind::InvalidResponse));
                }
                Ok((pull.number, pull))
            },
        )
        .await
    }

    pub async fn pull(
        &self,
        repository: &RepositoryId,
        number: u64,
    ) -> Result<PullRequest, ApiError> {
        match self.pull_if(repository, number, None).await? {
            Fresh::New(pull, _) => Ok(pull),
            Fresh::Same => Err(error(ApiErrorKind::InvalidResponse)),
        }
    }

    /// The pull request, asked with the validator of the copy held. A 304
    /// proves that copy current, so it becomes this client's head proof.
    pub async fn pull_if(
        &self,
        repository: &RepositoryId,
        number: u64,
        known: Option<(&PullRequest, &Validator)>,
    ) -> Result<Fresh<PullRequest, Option<Validator>>, ApiError> {
        let id = self.allowed(repository)?;
        if number == 0 {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        // A failed refresh cannot leave an older proof usable on this client.
        *self
            .verified_head
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))? = None;
        let known = known.filter(|(pull, _)| {
            pull.repository == id && pull.number == number && valid_sha(&pull.head_sha)
        });
        let path = format!("/repos/{}/pulls/{number}", id.full_name());
        let (pull, fresh) = match self
            .read_if::<RawPull>(&path, "", known.map(|(_, v)| v))
            .await?
        {
            Some((raw, headers)) => {
                let pull = raw.checked(&id, Some(number))?;
                (pull.clone(), Fresh::New(pull, validator(&headers)))
            }
            None => (
                known
                    .map(|(pull, _)| pull.clone())
                    .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?,
                Fresh::Same,
            ),
        };
        *self
            .verified_head
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))? = Some(HeadProof::from(&pull));
        Ok(fresh)
    }

    /// Read the exact PR head SHA in its base repository, then its distinct
    /// source repository. Every row retains repository provenance. These reads
    /// do not cover merge-ref workflows, branch protection, or merge readiness.
    pub async fn checks(&self, pull: &PullRequest) -> Result<Checks, ApiError> {
        match self.checks_if(pull, None).await? {
            Fresh::New(checks, _) => Ok(checks),
            Fresh::Same => Err(error(ApiErrorKind::InvalidResponse)),
        }
    }

    /// [`Client::checks`], each list asked with the validators of a complete
    /// read of the same head (`known`). Every list a 304 is `Same`; a list
    /// that is a 304 beside one that changed keeps the rows held for it.
    pub async fn checks_if(
        &self,
        pull: &PullRequest,
        known: Option<(&Checks, &CheckTags)>,
    ) -> Result<Fresh<Checks, CheckTags>, ApiError> {
        self.allowed(&pull.repository)?;
        let proof = HeadProof::from(pull);
        if self
            .verified_head
            .lock()
            .map_err(|_| error(ApiErrorKind::Unavailable))?
            .as_ref()
            != Some(&proof)
        {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        if !valid_sha(&proof.sha) {
            return Err(error(ApiErrorKind::InvalidTarget));
        }
        let mut scopes = vec![(proof.base.clone(), "base repository")];
        if let Some(head) = &proof.head {
            head.validated()?;
            if head != &proof.base {
                scopes.push((head.clone(), "source repository"));
            }
        }
        let mut check_runs = Collection {
            items: vec![],
            completion: Completion::complete(0),
        };
        let mut statuses = Collection {
            items: vec![],
            completion: Completion::complete(0),
        };
        let mut errors = vec![];
        let mut stopped = false;
        let skipped = "not requested after authentication or rate-limit refusal";
        // Validators describe only a complete read of this same head.
        let known = known.filter(|(checks, _)| {
            checks.head_sha == proof.sha
                && checks.head_repository == proof.head
                && checks.errors.is_empty()
                && checks.check_runs.completion.complete
                && checks.statuses.completion.complete
        });
        let mut tags = CheckTags::new();
        let mut same = known.is_some();
        for (repository, label) in scopes {
            let path = format!("/repos/{}/commits/{}", repository.full_name(), proof.sha);
            let runs_key = format!("{}/check-runs", repository.full_name());
            let statuses_key = format!("{}/statuses", repository.full_name());
            let tags_of = |key: &str| {
                known
                    .and_then(|(_, tags)| tags.get(key))
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
            };
            let held_runs = known
                .map(|(checks, _)| {
                    checks
                        .check_runs
                        .items
                        .iter()
                        .filter(|run| run.repository == repository)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let held_statuses = known
                .map(|(checks, _)| {
                    checks
                        .statuses
                        .items
                        .iter()
                        .filter(|status| status.repository == repository)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let scope_checks = if stopped {
                same = false;
                Collection::unavailable(skipped)
            } else {
                let fresh = self
                    .collect_if(
                        &format!("{path}/check-runs"),
                        "filter=latest",
                        Some("check_runs"),
                        tags_of(&runs_key),
                        |raw: RawCheckRun| {
                            if raw.id == 0
                                || !valid_sha(&raw.head_sha)
                                || !raw.head_sha.eq_ignore_ascii_case(&proof.sha)
                            {
                                return Err(error(ApiErrorKind::InvalidResponse));
                            }
                            Ok((
                                raw.id,
                                CheckRun {
                                    repository: repository.clone(),
                                    id: raw.id,
                                    name: raw.name,
                                    status: raw.status,
                                    conclusion: raw.conclusion,
                                    head_sha: proof.sha.clone(),
                                },
                            ))
                        },
                    )
                    .await;
                let known_runs = tags_of(&runs_key);
                reuse(runs_key, known_runs, fresh, held_runs, &mut tags, &mut same)
            };
            stopped |= scope_checks.completion.error.is_some_and(stops_reads);
            merge_collection(
                &mut check_runs,
                scope_checks,
                &format!("{label} check runs"),
                &mut errors,
            );
            let mut scope_statuses = if stopped {
                same = false;
                Collection::unavailable(skipped)
            } else {
                let fresh = self
                    .collect_if(
                        &format!("{path}/statuses"),
                        "",
                        None,
                        tags_of(&statuses_key),
                        |raw: RawStatus| {
                            if raw.id == 0
                                || !valid_status_url(&self.api, &raw.url, &repository, &proof.sha)
                            {
                                return Err(error(ApiErrorKind::InvalidResponse));
                            }
                            Ok((
                                raw.id,
                                CommitStatus {
                                    repository: repository.clone(),
                                    id: raw.id,
                                    context: raw.context,
                                    state: raw.state,
                                    description: raw.description,
                                },
                            ))
                        },
                    )
                    .await;
                let known_statuses = tags_of(&statuses_key);
                reuse(
                    statuses_key,
                    known_statuses,
                    fresh,
                    held_statuses,
                    &mut tags,
                    &mut same,
                )
            };
            stopped |= scope_statuses.completion.error.is_some_and(stops_reads);
            // Newest first, case-insensitive contexts; dedupe only within this repository.
            let mut contexts = HashSet::new();
            scope_statuses
                .items
                .retain(|status| contexts.insert(status.context.to_ascii_lowercase()));
            merge_collection(
                &mut statuses,
                scope_statuses,
                &format!("{label} commit statuses"),
                &mut errors,
            );
        }
        if proof.head.is_none() {
            let reason = "head repository is unavailable; source checks are unknown";
            merge_collection(
                &mut check_runs,
                Collection::unavailable(reason),
                "source repository check runs",
                &mut errors,
            );
            merge_collection(
                &mut statuses,
                Collection::unavailable(reason),
                "source repository commit statuses",
                &mut errors,
            );
        }
        if same {
            return Ok(Fresh::Same);
        }
        Ok(Fresh::New(
            Checks {
                head_sha: proof.sha,
                head_repository: proof.head,
                check_runs,
                statuses,
                errors,
            },
            tags,
        ))
    }

    /// What moved, for many repositories in one GraphQL query per
    /// [`PROBE_BATCH`]: each one's open pull request count and newest update.
    /// `None` for a repository GitHub did not describe (missing, renamed or
    /// denied), which is then read over REST. Needs a token.
    pub async fn markers(
        &self,
        repositories: &[RepositoryId],
    ) -> Result<Vec<(RepositoryId, Option<Marker>)>, ApiError> {
        let asks: Vec<_> = repositories.iter().map(|id| (id.clone(), false)).collect();
        Ok(self
            .probe(&asks)
            .await?
            .into_iter()
            .map(|probed| (probed.id, probed.marker))
            .collect())
    }

    /// [`Client::markers`], and for each repository asked for in full
    /// (`true`: nothing is held for it yet) its description and open pull
    /// requests too, which saves the two REST reads it would cost. The
    /// batches go out [`PROBE_CONCURRENCY`] at a time; the first refusal
    /// ends the probe, abandoning those still in flight.
    pub async fn probe(&self, asks: &[(RepositoryId, bool)]) -> Result<Vec<Probed>, ApiError> {
        if self.token_ref.is_empty() {
            return Err(error(ApiErrorKind::MissingToken));
        }
        let asks = asks
            .iter()
            .map(|(id, full)| Ok((self.allowed(id)?, *full)))
            .collect::<Result<Vec<_>, ApiError>>()?;
        // Full reads weigh more than markers, so they go in smaller batches,
        // and first, being the slowest to answer.
        let (heavy, light): (Vec<usize>, Vec<usize>) = (0..asks.len()).partition(|i| asks[*i].1);
        let mut groups = heavy
            .chunks(FULL_BATCH)
            .chain(light.chunks(PROBE_BATCH))
            .map(<[usize]>::to_vec);
        let mut answers: Vec<Option<Probed>> = vec![None; asks.len()];
        let mut running = tokio::task::JoinSet::new();
        loop {
            while running.len() < PROBE_CONCURRENCY
                && let Some(members) = groups.next()
            {
                let client = self.clone();
                let batch: Vec<_> = members.iter().map(|i| asks[*i].clone()).collect();
                running.spawn(async move { (members, client.probe_batch(&batch).await) });
            }
            let Some(joined) = running.join_next().await else {
                break;
            };
            let (members, answer) = joined.map_err(|_| error(ApiErrorKind::Unavailable))?;
            for (i, probed) in members.into_iter().zip(answer?) {
                answers[i] = Some(probed);
            }
        }
        Ok(answers.into_iter().flatten().collect())
    }

    async fn probe_batch(&self, batch: &[(RepositoryId, bool)]) -> Result<Vec<Probed>, ApiError> {
        let mut declared = Vec::with_capacity(batch.len());
        let mut fields = String::new();
        let mut variables = serde_json::Map::new();
        for (i, (id, full)) in batch.iter().enumerate() {
            declared.push(format!("$o{i}:String!,$n{i}:String!"));
            if *full {
                fields.push_str(&format!(
                    "r{i}:repository(owner:$o{i},name:$n{i}){{nameWithOwner description \
                     isPrivate isArchived defaultBranchRef{{name}} \
                     pullRequests(states:OPEN,first:{FULL_PULLS},\
                     orderBy:{{field:UPDATED_AT,direction:DESC}}){{totalCount nodes{{\
                     number title body isDraft headRefOid headRefName baseRefName updatedAt \
                     author{{login}} headRepository{{nameWithOwner}}}}}}}} "
                ));
            } else {
                fields.push_str(&format!(
                    "r{i}:repository(owner:$o{i},name:$n{i}){{nameWithOwner \
                     pullRequests(states:OPEN,first:1,orderBy:{{field:UPDATED_AT,direction:DESC}})\
                     {{totalCount nodes{{updatedAt}}}}}} "
                ));
            }
            variables.insert(format!("o{i}"), id.owner.clone().into());
            variables.insert(format!("n{i}"), id.name.clone().into());
        }
        let query = format!("query({}){{{}}}", declared.join(","), fields.trim_end());
        let body = serde_json::to_vec(&serde_json::json!({
            "query": query,
            "variables": variables,
        }))
        .map_err(|_| error(ApiErrorKind::InvalidTarget))?;
        let value = self.graphql(body).await?;
        let data = value
            .get("data")
            .filter(|data| data.is_object())
            .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?;
        Ok(batch
            .iter()
            .enumerate()
            .map(|(i, (id, full))| {
                let repo = data.get(format!("r{i}"));
                let marker = repo.and_then(|repo| marker(repo, id));
                let full = repo
                    .filter(|_| *full && marker.is_some())
                    .and_then(|repo| full_of(repo, id));
                Probed {
                    id: id.clone(),
                    marker,
                    full,
                }
            })
            .collect())
    }

    async fn graphql(&self, body: Vec<u8>) -> Result<serde_json::Value, ApiError> {
        let mut headers = base_headers();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let request = Request {
            headers,
            url: self.api.graphql().into(),
            authorization: auth::resolve_at(&self.token_ref, &self.api.gh_host())?,
            graphql: Some(body),
        };
        let response = self.transport.send(request).await?;
        self.note(&response);
        if response.status != 200 {
            return Err(response_error(&response));
        }
        let body = response.body.as_ref().map_err(Clone::clone)?;
        if body.len() > MAX_BODY_BYTES {
            return Err(error(ApiErrorKind::BodyTooLarge));
        }
        let value: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| error(ApiErrorKind::InvalidResponse))?;
        // GraphQL reports its own rate limit as an error in a 200.
        let limited = value
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|errors| {
                errors.iter().any(|e| {
                    e.get("type").and_then(serde_json::Value::as_str) == Some("RATE_LIMITED")
                })
            });
        if limited {
            let mut err = response_error(&Response {
                status: 429,
                headers: response.headers.clone(),
                body: Ok(vec![]),
            });
            err.status = Some(200);
            return Err(err);
        }
        Ok(value)
    }

    async fn read<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
    ) -> Result<(T, HeaderMap), ApiError> {
        self.read_if(path, query, None)
            .await?
            .ok_or_else(|| error(ApiErrorKind::InvalidResponse))
    }

    /// One GET, asked with `known` when given: `None` is GitHub's 304 for it.
    async fn read_if<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &str,
        known: Option<&Validator>,
    ) -> Result<Option<(T, HeaderMap)>, ApiError> {
        let mut headers = base_headers();
        let known = known.filter(|known| ask_if(known, &mut headers));
        let request = Request {
            headers,
            url: if query.is_empty() {
                format!("{}{path}", self.api.rest())
            } else {
                format!("{}{path}?{query}", self.api.rest())
            },
            authorization: auth::resolve_at(&self.token_ref, &self.api.gh_host())?,
            graphql: None,
        };
        let response = self.transport.send(request).await?;
        self.note(&response);
        if response.status == 304 {
            // Not modified: nothing to parse, the copy held is current.
            return if known.is_some() {
                Ok(None)
            } else {
                Err(error(ApiErrorKind::InvalidResponse))
            };
        }
        if response.status != 200 {
            return Err(response_error(&response));
        }
        let body = response.body?;
        if body.len() > MAX_BODY_BYTES {
            return Err(error(ApiErrorKind::BodyTooLarge));
        }
        let value =
            serde_json::from_slice(&body).map_err(|_| error(ApiErrorKind::InvalidResponse))?;
        Ok(Some((value, response.headers)))
    }

    async fn collect<R, T, F>(
        &self,
        path: &str,
        query: &str,
        envelope: Option<&str>,
        convert: F,
    ) -> Result<Collection<T>, ApiError>
    where
        R: DeserializeOwned,
        F: FnMut(R) -> Result<(u64, T), ApiError>,
    {
        match self.collect_if(path, query, envelope, &[], convert).await? {
            Fresh::New(collection, _) => Ok(collection),
            Fresh::Same => Err(error(ApiErrorKind::InvalidResponse)),
        }
    }

    /// Pages of a list. With `known` (one validator per page of the last
    /// complete read) every page is first asked conditionally: all 304 is
    /// `Same`. A changed first page is read on as usual; a later page that
    /// changed after unchanged ones starts the list again unconditionally,
    /// since the unchanged pages' rows are not held separately. Validators
    /// come back only for a complete read where every page carried one.
    async fn collect_if<R, T, F>(
        &self,
        path: &str,
        query: &str,
        envelope: Option<&str>,
        known: &[Validator],
        convert: F,
    ) -> Result<Fresh<Collection<T>>, ApiError>
    where
        R: DeserializeOwned,
        F: FnMut(R) -> Result<(u64, T), ApiError>,
    {
        self.collect_paged(path, query, envelope, known, Paging::ALL, convert)
            .await
    }

    /// [`Client::collect_if`] with its own page size and bound. A bounded
    /// list stops at `max_pages` by design and is complete for what was
    /// asked, saying so in its reason.
    async fn collect_paged<R, T, F>(
        &self,
        path: &str,
        query: &str,
        envelope: Option<&str>,
        known: &[Validator],
        paging: Paging,
        mut convert: F,
    ) -> Result<Fresh<Collection<T>>, ApiError>
    where
        R: DeserializeOwned,
        F: FnMut(R) -> Result<(u64, T), ApiError>,
    {
        let per_page = paging.per_page;
        let page_query = |page: u32| {
            if query.is_empty() {
                format!("per_page={per_page}&page={page}")
            } else {
                format!("{query}&per_page={per_page}&page={page}")
            }
        };
        let mut first = None;
        if !known.is_empty() {
            let mut moved = false;
            for (index, validator) in known.iter().enumerate() {
                let page = index as u32 + 1;
                match self
                    .read_if::<serde_json::Value>(path, &page_query(page), Some(validator))
                    .await?
                {
                    None => {}
                    Some(response) if index == 0 => {
                        first = Some(response);
                        break;
                    }
                    Some(_) => {
                        moved = true;
                        break;
                    }
                }
            }
            if first.is_none() && !moved {
                return Ok(Fresh::Same);
            }
        }
        let done = |items, completion: Completion, validators: Vec<Validator>, tagged| {
            let validators = if completion.complete && tagged {
                validators
            } else {
                vec![]
            };
            Ok(Fresh::New(Collection { items, completion }, validators))
        };
        let mut items = vec![];
        let mut identities = HashSet::new();
        let mut pages = 0;
        let mut changed = false;
        let mut validators = vec![];
        let mut tagged = true;
        loop {
            let page = pages + 1;
            let response = match first.take() {
                Some(response) => Ok(response),
                None => {
                    self.read::<serde_json::Value>(path, &page_query(page))
                        .await
                }
            };
            let (value, headers) = match response {
                Ok(response) => response,
                Err(err) if pages == 0 => return Err(err),
                Err(err) => {
                    return done(items, Completion::failed(pages, &err), validators, false);
                }
            };
            match validator(&headers) {
                Some(validator) => validators.push(validator),
                None => tagged = false,
            }
            let (values, total) = match decode_page::<R>(value, envelope) {
                Ok(page) => page,
                Err(err) if pages == 0 => return Err(err),
                Err(err) => {
                    return done(items, Completion::failed(pages, &err), validators, false);
                }
            };
            if values.len() > per_page {
                let err = error(ApiErrorKind::InvalidResponse);
                if pages == 0 {
                    return Err(err);
                }
                return done(items, Completion::failed(pages, &err), validators, false);
            }
            let mut staged = vec![];
            let mut staged_ids = HashSet::new();
            for value in values {
                let (identity, value) = match convert(value) {
                    Ok(value) => value,
                    Err(err) if pages == 0 => return Err(err),
                    Err(err) => {
                        return done(items, Completion::failed(pages, &err), validators, false);
                    }
                };
                if !identities.contains(&identity) && staged_ids.insert(identity) {
                    staged.push(value);
                } else {
                    changed = true;
                }
            }
            identities.extend(staged_ids);
            items.extend(staged);
            pages = page;
            let next = match next_page(&self.api, &headers, path, page) {
                Ok(next) => next,
                Err(()) => {
                    return done(
                        items,
                        Completion::partial(pages, INVALID_PAGINATION),
                        validators,
                        false,
                    );
                }
            };
            if !next {
                let incomplete = changed || total.is_some_and(|total| total != items.len() as u64);
                let completion = if incomplete {
                    Completion::partial(pages, CHANGED_PAGES)
                } else {
                    Completion::complete(pages)
                };
                return done(items, completion, validators, tagged);
            }
            if pages >= paging.max_pages {
                if paging.bounded && !changed {
                    let completion = Completion::bounded(pages, items.len());
                    return done(items, completion, validators, tagged);
                }
                return done(
                    items,
                    Completion::partial(pages, PAGE_CAP),
                    validators,
                    false,
                );
            }
        }
    }
}

/// How a list is paged: the rows a page holds, how many pages are read, and
/// whether stopping there is the ask (not a cap that hides rows).
#[derive(Clone, Copy)]
struct Paging {
    per_page: usize,
    max_pages: u32,
    bounded: bool,
}

impl Paging {
    const ALL: Paging = Paging {
        per_page: PER_PAGE,
        max_pages: MAX_PAGES,
        bounded: false,
    };
}

/// Rows held for a check list that GitHub answered 304, or the new read.
fn reuse<T>(
    key: String,
    known: &[Validator],
    fresh: Result<Fresh<Collection<T>>, ApiError>,
    held: Vec<T>,
    tags: &mut CheckTags,
    same: &mut bool,
) -> Collection<T> {
    match fresh {
        Ok(Fresh::Same) => {
            tags.insert(key, known.to_vec());
            Collection {
                items: held,
                completion: Completion::complete(known.len() as u32),
            }
        }
        Ok(Fresh::New(collection, validators)) => {
            *same = false;
            if !validators.is_empty() {
                tags.insert(key, validators);
            }
            collection
        }
        Err(err) => {
            *same = false;
            result_collection(Err(err))
        }
    }
}

fn base_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "accept",
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "x-github-api-version",
        HeaderValue::from_static(API_VERSION),
    );
    headers.insert(
        "user-agent",
        HeaderValue::from_static("radar-github-read-only/0.1"),
    );
    headers
}

/// The scopes a classic or OAuth token has, from `X-OAuth-Scopes`
/// (`repo, read:org`); empty when it has none. `None` when GitHub sent no
/// such header, as for a fine-grained token.
fn oauth_scopes(headers: &HeaderMap) -> Option<Vec<String>> {
    let value = headers.get("x-oauth-scopes")?.to_str().ok()?;
    if value.len() > 1024 {
        return None;
    }
    Some(value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
}

/// A validator value GitHub sent: printable ASCII, bounded.
fn header_text(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    (!value.is_empty() && value.len() <= 256 && value.bytes().all(|b| (b' '..=b'~').contains(&b)))
        .then(|| value.to_string())
}

fn validator(headers: &HeaderMap) -> Option<Validator> {
    let validator = Validator {
        etag: header_text(headers, ETAG),
        last_modified: header_text(headers, LAST_MODIFIED),
    };
    (validator.etag.is_some() || validator.last_modified.is_some()).then_some(validator)
}

/// Ask conditionally with what describes the copy held: the entity tag when
/// there is one, else the date. False when neither can be sent.
fn ask_if(known: &Validator, headers: &mut HeaderMap) -> bool {
    let value = |text: &Option<String>| {
        text.as_deref()
            .filter(|t| t.len() <= 256 && t.bytes().all(|b| (b' '..=b'~').contains(&b)))
            .and_then(|t| HeaderValue::from_str(t).ok())
    };
    if let Some(etag) = value(&known.etag) {
        headers.insert(IF_NONE_MATCH, etag);
        true
    } else if let Some(date) = value(&known.last_modified) {
        headers.insert(IF_MODIFIED_SINCE, date);
        true
    } else {
        false
    }
}

/// `x-ratelimit-*` of one response, when GitHub sent all of them.
fn rate_limit(headers: &HeaderMap) -> Option<RateLimit> {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let resource = headers
        .get("x-ratelimit-resource")
        .and_then(|v| v.to_str().ok())
        .filter(|r| {
            !r.is_empty()
                && r.len() <= 32
                && r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .unwrap_or("core");
    Some(RateLimit {
        resource: resource.into(),
        limit: number("x-ratelimit-limit")?,
        remaining: number("x-ratelimit-remaining")?,
        reset: i64::try_from(number("x-ratelimit-reset")?).ok()?,
    })
}

/// The marker of one repository in a GraphQL answer, if it is the one asked.
fn marker(repo: &serde_json::Value, id: &RepositoryId) -> Option<Marker> {
    let name = repo.get("nameWithOwner")?.as_str()?;
    if RepositoryId::parse(name).ok().as_ref() != Some(id) {
        return None;
    }
    let pulls = repo.get("pullRequests")?;
    let open = pulls.get("totalCount")?.as_u64()?;
    let latest = match pulls.get("nodes")?.as_array()?.first() {
        Some(node) => {
            let at = node.get("updatedAt")?.as_str()?;
            if at.len() > 40 || !at.bytes().all(|b| b.is_ascii_graphic()) {
                return None;
            }
            Some(at.to_string())
        }
        None => None,
    };
    Some(Marker { open, latest })
}

/// A repository's description and open pull requests from a GraphQL answer,
/// exactly as the REST reads would give them; `None` when anything is off or
/// not all of its pull requests came (then REST reads it).
fn full_of(repo: &serde_json::Value, id: &RepositoryId) -> Option<Full> {
    let pulls = repo.get("pullRequests")?;
    let nodes = pulls.get("nodes")?.as_array()?;
    if pulls.get("totalCount")?.as_u64()? != nodes.len() as u64 {
        return None;
    }
    let repository = Repository {
        id: id.clone(),
        description: repo
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        default_branch: repo
            .get("defaultBranchRef")?
            .get("name")?
            .as_str()
            .filter(|branch| !branch.is_empty())?
            .to_string(),
        private: repo.get("isPrivate")?.as_bool()?,
        archived: repo.get("isArchived")?.as_bool()?,
    };
    let items = nodes
        .iter()
        .map(|node| pull_of(node, id))
        .collect::<Option<Vec<_>>>()?;
    let mut numbers = HashSet::new();
    if !items.iter().all(|pull| numbers.insert(pull.number)) {
        return None;
    }
    Some(Full {
        repository,
        pulls: Collection {
            items,
            completion: Completion::complete(1),
        },
    })
}

fn pull_of(node: &serde_json::Value, id: &RepositoryId) -> Option<PullRequest> {
    let text = |field: &str| node.get(field)?.as_str().map(str::to_string);
    let sha = text("headRefOid").filter(|sha| valid_sha(sha))?;
    let updated_at = text("updatedAt")
        .filter(|at| at.len() <= 40 && at.bytes().all(|b| b.is_ascii_graphic()))?;
    let head_repository = match node.get("headRepository")? {
        serde_json::Value::Null => None,
        head => Some(RepositoryId::parse(head.get("nameWithOwner")?.as_str()?).ok()?),
    };
    let author = match node.get("author")? {
        serde_json::Value::Null => "unknown".to_string(),
        author => author.get("login")?.as_str()?.to_string(),
    };
    Some(PullRequest {
        repository: id.clone(),
        number: node.get("number")?.as_u64().filter(|n| *n > 0)?,
        title: text("title")?,
        // REST says null for an empty description, GraphQL says "".
        body: text("body").filter(|body| !body.is_empty()),
        author,
        state: "open".into(),
        draft: node.get("isDraft")?.as_bool()?,
        head_sha: sha.to_ascii_lowercase(),
        head_repository,
        head_branch: text("headRefName")?,
        base_branch: text("baseRefName")?,
        updated_at,
        merged: false,
    })
}

fn result_collection<T>(result: Result<Collection<T>, ApiError>) -> Collection<T> {
    match result {
        Ok(collection) => collection,
        Err(err) => Collection {
            items: vec![],
            completion: Completion::failed(0, &err),
        },
    }
}

fn stops_reads(kind: ApiErrorKind) -> bool {
    matches!(
        kind,
        ApiErrorKind::Authentication
            | ApiErrorKind::RateLimit
            | ApiErrorKind::MissingToken
            | ApiErrorKind::InvalidToken
    )
}

fn merge_collection<T>(
    target: &mut Collection<T>,
    source: Collection<T>,
    label: &str,
    errors: &mut Vec<String>,
) {
    target.items.extend(source.items);
    target.completion.pages += source.completion.pages;
    if !source.completion.complete {
        target.completion.complete = false;
        let reason = format!(
            "{label}: {}",
            source
                .completion
                .reason
                .as_deref()
                .unwrap_or("read was incomplete")
        );
        errors.push(reason.clone());
        match &mut target.completion.reason {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(&reason);
            }
            None => target.completion.reason = Some(reason),
        }
    }
    if target.completion.error.is_none() || source.completion.error.is_some_and(stops_reads) {
        target.completion.error = source.completion.error.or(target.completion.error);
    }
}

fn decode_page<R: DeserializeOwned>(
    mut value: serde_json::Value,
    envelope: Option<&str>,
) -> Result<(Vec<R>, Option<u64>), ApiError> {
    let total = if envelope.is_some() {
        Some(
            value
                .get("total_count")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?,
        )
    } else {
        None
    };
    if let Some(field) = envelope {
        value = value
            .get_mut(field)
            .ok_or_else(|| error(ApiErrorKind::InvalidResponse))?
            .take();
    }
    let items = serde_json::from_value(value).map_err(|_| error(ApiErrorKind::InvalidResponse))?;
    Ok((items, total))
}

/// Inspect continuation evidence, then reconstruct our own request to the configured API.
/// Numeric repository aliases are accepted only for the same endpoint suffix.
fn next_page(api: &Endpoint, headers: &HeaderMap, path: &str, page: u32) -> Result<bool, ()> {
    let mut found = false;
    for header in headers.get_all(LINK) {
        let header = header.to_str().map_err(|_| ())?;
        for link in header.split(',') {
            let mut segments = link.trim().split(';');
            let target = segments.next().ok_or(())?.trim();
            let mut relation = None;
            for parameter in segments {
                let (key, value) = parameter.trim().split_once('=').ok_or(())?;
                if key == "rel" {
                    relation = Some(value.trim_matches('"'));
                }
            }
            let relation = relation.ok_or(())?;
            if !relation.split_ascii_whitespace().any(|name| name == "next") {
                continue;
            }
            if found {
                return Err(());
            }
            let raw = target
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix('>'))
                .ok_or(())?;
            if raw.contains(['\\', '%']) || raw.chars().any(char::is_whitespace) {
                return Err(());
            }
            let url = url::Url::parse(raw).map_err(|_| ())?;
            // Same scheme, host and port as the configured API, so a token is
            // never carried to another host; the path is the API's own.
            if !api.is_origin_of(&url) {
                return Err(());
            }
            let Some(url_path) = url.path().strip_prefix(api.base_path()) else {
                return Err(());
            };
            let suffix = path.splitn(5, '/').nth(4).ok_or(())?;
            let alias_ok = url_path
                .strip_prefix("/repositories/")
                .and_then(|s| s.split_once('/'))
                .is_some_and(|(id, tail)| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && tail == suffix
                });
            if url_path != path && !alias_ok {
                return Err(());
            }
            let pages: Vec<_> = url.query_pairs().filter(|(key, _)| key == "page").collect();
            if pages.len() != 1 || pages[0].1.parse::<u32>().ok() != Some(page + 1) {
                return Err(());
            }
            found = true;
        }
    }
    Ok(found)
}

/// A GET, or with `graphql` a POST of that query to [`Endpoint::graphql`] only.
struct Request {
    url: String,
    headers: HeaderMap,
    authorization: Option<HeaderValue>,
    graphql: Option<Vec<u8>>,
}
struct Response {
    status: u16,
    headers: HeaderMap,
    body: Result<Vec<u8>, ApiError>,
}
type ResponseFuture<'a> = Pin<Box<dyn Future<Output = Result<Response, ApiError>> + Send + 'a>>;
trait Transport: Send + Sync {
    fn send(&self, request: Request) -> ResponseFuture<'_>;
}

struct NativeTransport {
    http: reqwest::Client,
    graphql: String,
}
impl NativeTransport {
    fn new(api: &Endpoint) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            // https unless the configured API is itself plain http, as a
            // local server is.
            .https_only(api.is_https())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent("radar-github-read-only/0.1")
            .build()
            .map_err(|_| error(ApiErrorKind::Network))?;
        Ok(Self {
            http,
            graphql: api.graphql().into(),
        })
    }
}
impl Transport for NativeTransport {
    fn send(&self, request: Request) -> ResponseFuture<'_> {
        Box::pin(async move {
            let mut builder = match request.graphql {
                None => self.http.get(request.url),
                Some(body) if request.url == self.graphql => {
                    self.http.post(&self.graphql).body(body)
                }
                Some(_) => return Err(error(ApiErrorKind::InvalidTarget)),
            }
            .headers(request.headers);
            if let Some(value) = request.authorization {
                builder = builder.header(AUTHORIZATION, value);
            }
            let mut response = builder
                .send()
                .await
                .map_err(|_| error(ApiErrorKind::Network))?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let body = if response
                .content_length()
                .is_some_and(|len| len > MAX_BODY_BYTES as u64)
            {
                Err(error(ApiErrorKind::BodyTooLarge))
            } else {
                let mut bytes = vec![];
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= MAX_BODY_BYTES => {
                            bytes.extend_from_slice(&chunk)
                        }
                        Ok(Some(_)) => break Err(error(ApiErrorKind::BodyTooLarge)),
                        Ok(None) => break Ok(bytes),
                        Err(_) => break Err(error(ApiErrorKind::Body)),
                    }
                }
            };
            Ok(Response {
                status,
                headers,
                body,
            })
        })
    }
}

fn response_error(response: &Response) -> ApiError {
    let numeric = |name: &str| {
        response
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    let secondary_limit = response
        .body
        .as_ref()
        .ok()
        .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
        .and_then(|value| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::to_ascii_lowercase)
        })
        .is_some_and(|message| message.contains("rate limit"));
    let kind = match response.status {
        401 => ApiErrorKind::Authentication,
        429 => ApiErrorKind::RateLimit,
        403 if numeric("x-ratelimit-remaining") == Some(0)
            || numeric("retry-after").is_some()
            || secondary_limit =>
        {
            ApiErrorKind::RateLimit
        }
        403 => ApiErrorKind::Forbidden,
        404 => ApiErrorKind::NotFound,
        301 | 302 | 303 | 307 | 308 => ApiErrorKind::Redirect,
        500..=599 => ApiErrorKind::Unavailable,
        _ => ApiErrorKind::InvalidResponse,
    };
    ApiError {
        kind,
        status: Some(response.status),
        retry_after_secs: numeric("retry-after"),
        rate_limit_reset: numeric("x-ratelimit-reset"),
    }
}

fn error(kind: ApiErrorKind) -> ApiError {
    ApiError::new(kind)
}
fn valid_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn valid_repository_url(value: &str, repository: &RepositoryId) -> bool {
    value
        .strip_prefix("https://github.com/")
        .and_then(|path| RepositoryId::parse(path).ok())
        .as_ref()
        == Some(repository)
}
fn valid_status_url(api: &Endpoint, value: &str, repository: &RepositoryId, sha: &str) -> bool {
    let Some(path) = value
        .strip_prefix(api.rest())
        .and_then(|rest| rest.strip_prefix("/repos/"))
    else {
        return false;
    };
    let Some((name, tail)) = path.split_once("/statuses/") else {
        return false;
    };
    RepositoryId::parse(name).ok().as_ref() == Some(repository) && tail.eq_ignore_ascii_case(sha)
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
}

#[derive(Deserialize)]
struct RawAccount {
    id: u64,
    login: String,
}
#[derive(Deserialize)]
struct RawRepository {
    #[serde(default)]
    id: Option<u64>,
    full_name: String,
    name: String,
    owner: RawUser,
    html_url: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    private: Option<bool>,
    #[serde(default)]
    archived: Option<bool>,
}
impl RawRepository {
    fn identity(&self) -> Result<RepositoryId, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        let id = RepositoryId::parse(&self.full_name).map_err(|_| bad())?;
        let fields = RepositoryId::parse(&format!("{}/{}", self.owner.login, self.name))
            .map_err(|_| bad())?;
        if id != fields || !valid_repository_url(&self.html_url, &id) {
            return Err(bad());
        }
        Ok(id)
    }
}
#[derive(Deserialize)]
struct RawBranch {
    sha: String,
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<RawRepository>,
}
#[derive(Deserialize)]
struct RawPull {
    number: u64,
    html_url: String,
    title: String,
    body: Option<String>,
    user: Option<RawUser>,
    state: String,
    draft: bool,
    head: RawBranch,
    base: RawBranch,
    updated_at: String,
    merged: Option<bool>,
}
impl RawPull {
    fn checked(
        self,
        repository: &RepositoryId,
        number: Option<u64>,
    ) -> Result<PullRequest, ApiError> {
        let bad = || error(ApiErrorKind::InvalidResponse);
        if self.number == 0
            || number.is_some_and(|number| number != self.number)
            || !valid_sha(&self.head.sha)
            || !valid_sha(&self.base.sha)
        {
            return Err(bad());
        }
        let base = self.base.repo.as_ref().ok_or_else(bad)?.identity()?;
        let head = self
            .head
            .repo
            .as_ref()
            .map(RawRepository::identity)
            .transpose()?;
        let Some((url_repo, url_number)) = self
            .html_url
            .strip_prefix("https://github.com/")
            .and_then(|path| path.split_once("/pull/"))
        else {
            return Err(bad());
        };
        if &base != repository
            || RepositoryId::parse(url_repo).ok().as_ref() != Some(repository)
            || url_number != self.number.to_string()
            || (number.is_some() && self.merged.is_none())
        {
            return Err(bad());
        }
        Ok(PullRequest {
            repository: repository.clone(),
            number: self.number,
            title: self.title,
            body: self.body,
            author: self
                .user
                .map(|user| user.login)
                .unwrap_or_else(|| "unknown".into()),
            state: self.state,
            draft: self.draft,
            head_sha: self.head.sha.to_ascii_lowercase(),
            head_repository: head,
            head_branch: self.head.branch,
            base_branch: self.base.branch,
            updated_at: self.updated_at,
            merged: self.merged.unwrap_or(false),
        })
    }
}
#[derive(Deserialize)]
struct RawCheckRun {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    head_sha: String,
}
#[derive(Deserialize)]
struct RawStatus {
    id: u64,
    url: String,
    context: String,
    state: String,
    description: Option<String>,
}

#[cfg(test)]
mod tests;
